//! Docker's stats messages turned into one second of usage: CPU by the formula `docker stats`
//! uses, memory without the page cache, and per-second rates from the cumulative disk and network
//! counters of two consecutive messages.
use serde_json::Value;

/// The counters of one stats message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reading {
    /// When Docker read the counters, in Unix nanoseconds.
    pub read: i128,
    /// CPU time of the container and of the whole host, now and at Docker's previous reading.
    pub cpu: u64,
    pub precpu: u64,
    pub system: u64,
    pub presystem: u64,
    pub online_cpus: u32,
    pub memory: u64,
    pub memory_limit: u64,
    pub disk_read: u64,
    pub disk_write: u64,
    pub net_rx: u64,
    pub net_tx: u64,
}

/// What one interval measured, ready to be stamped and stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Measure {
    pub cpu: f32,
    pub memory: u64,
    pub memory_limit: u64,
    pub online_cpus: u32,
    pub disk_read: f32,
    pub disk_write: f32,
    pub net_rx: f32,
    pub net_tx: f32,
}

/// What `/metrics` reports of a container from its latest stats message: Docker's counters since
/// the container started, and its CPU and memory at that moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Counters {
    /// CPU time since the container started, in nanoseconds.
    pub cpu_time: u64,
    /// Cores in use since Docker's previous reading, as [`cores`] counts them.
    pub cpu: f32,
    pub memory: u64,
    pub memory_limit: u64,
    pub disk_read: u64,
    pub disk_write: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    /// Processes and threads, where Docker counts them.
    pub pids: Option<u64>,
}

/// The counters of `reading`, with the processes its `message` names.
pub(crate) fn counters(reading: &Reading, message: &Value) -> Counters {
    Counters {
        cpu_time: reading.cpu,
        cpu: cores(reading),
        memory: reading.memory,
        memory_limit: reading.memory_limit,
        disk_read: reading.disk_read,
        disk_write: reading.disk_write,
        net_rx: reading.net_rx,
        net_tx: reading.net_tx,
        pids: message["pids_stats"]["current"].as_u64(),
    }
}

/// The counters of a message, or `None` for the empty one Docker sends while a container is not
/// running, which carries no host CPU time.
pub(crate) fn reading(message: &Value) -> Option<Reading> {
    let cpu = &message["cpu_stats"];
    let precpu = &message["precpu_stats"];
    let system = cpu["system_cpu_usage"]
        .as_u64()
        .filter(|value| *value > 0)?;
    let read = unix_nanos(message["read"].as_str()?)?;
    let online_cpus = cpu["online_cpus"]
        .as_u64()
        .filter(|value| *value > 0)
        .or_else(|| {
            cpu["cpu_usage"]["percpu_usage"]
                .as_array()
                .map(|cpus| cpus.len() as u64)
        })
        .unwrap_or(1);
    let memory = &message["memory_stats"];
    let usage = memory["usage"].as_u64().unwrap_or_default();
    // The page cache Linux can drop is not counted: cgroup v1 names it total_inactive_file and
    // cgroup v2 inactive_file.
    let cache = memory["stats"]["total_inactive_file"]
        .as_u64()
        .or(memory["stats"]["inactive_file"].as_u64())
        .filter(|cache| *cache < usage)
        .unwrap_or_default();
    let (mut disk_read, mut disk_write) = (0, 0);
    for entry in message["blkio_stats"]["io_service_bytes_recursive"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let value = entry["value"].as_u64().unwrap_or_default();
        match entry["op"].as_str().and_then(|op| op.bytes().next()) {
            Some(b'r' | b'R') => disk_read += value,
            Some(b'w' | b'W') => disk_write += value,
            _ => {}
        }
    }
    let (mut net_rx, mut net_tx) = (0, 0);
    for network in message["networks"].as_object().into_iter().flatten() {
        net_rx += network.1["rx_bytes"].as_u64().unwrap_or_default();
        net_tx += network.1["tx_bytes"].as_u64().unwrap_or_default();
    }
    Some(Reading {
        read,
        cpu: cpu["cpu_usage"]["total_usage"].as_u64().unwrap_or_default(),
        precpu: precpu["cpu_usage"]["total_usage"]
            .as_u64()
            .unwrap_or_default(),
        system,
        presystem: precpu["system_cpu_usage"].as_u64().unwrap_or_default(),
        online_cpus: u32::try_from(online_cpus).unwrap_or(u32::MAX),
        memory: usage - cache,
        memory_limit: memory["limit"].as_u64().unwrap_or_default(),
        disk_read,
        disk_write,
        net_rx,
        net_tx,
    })
}

/// CPU cores in use since Docker's previous reading, which `docker stats` shows times 100 as its
/// CPU %. It is measured against the host's CPUs, not the container's limit: a container limited
/// to two CPUs reads at most 2.0.
pub(crate) fn cores(reading: &Reading) -> f32 {
    let cpu = reading.cpu.saturating_sub(reading.precpu) as f64;
    let system = reading.system.saturating_sub(reading.presystem) as f64;
    // The first message of a stream has no previous reading to measure against.
    if reading.presystem == 0 || system <= 0.0 {
        return 0.0;
    }
    (cpu / system * f64::from(reading.online_cpus)) as f32
}

/// The second between two consecutive readings of one container, or `None` when they do not
/// follow each other in time.
pub(crate) fn measure(previous: &Reading, current: &Reading) -> Option<Measure> {
    let seconds = (current.read - previous.read) as f64 / 1e9;
    if seconds <= 0.0 {
        return None;
    }
    // A counter that went back belongs to a restarted container; nothing is known to have moved.
    let rate = |now: u64, before: u64| (now.saturating_sub(before) as f64 / seconds) as f32;
    Some(Measure {
        cpu: cores(current),
        memory: current.memory,
        memory_limit: current.memory_limit,
        online_cpus: current.online_cpus,
        disk_read: rate(current.disk_read, previous.disk_read),
        disk_write: rate(current.disk_write, previous.disk_write),
        net_rx: rate(current.net_rx, previous.net_rx),
        net_tx: rate(current.net_tx, previous.net_tx),
    })
}

/// Unix nanoseconds of an RFC 3339 time, as Docker writes `read`: `2026-10-16T09:30:00.123456789Z`,
/// or with the daemon's offset instead of `Z`.
pub(crate) fn unix_nanos(text: &str) -> Option<i128> {
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> {
        let digits = text.get(from..to)?;
        digits
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut rest = &text[19..];
    let mut nanos: i128 = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        // Digits past nanoseconds are dropped.
        for (place, digit) in fraction
            .bytes()
            .take(9)
            .take_while(u8::is_ascii_digit)
            .enumerate()
        {
            nanos += i128::from(digit - b'0') * 10i128.pow(8 - place as u32);
        }
        rest = &fraction[digits..];
    }
    let offset = match rest.as_bytes() {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let field = |a: &u8, b: &u8| {
                (a.is_ascii_digit() && b.is_ascii_digit())
                    .then(|| i64::from(a - b'0') * 10 + i64::from(b - b'0'))
            };
            let seconds = field(h1, h2)? * 3600 + field(m1, m2)? * 60;
            if *sign == b'-' { -seconds } else { seconds }
        }
        _ => return None,
    };
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Some(i128::from(seconds) * 1_000_000_000 + nanos)
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
