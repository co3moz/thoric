//! Pins down the numbers a stats message becomes: Docker's own CPU formula measured against the
//! host's CPUs, memory without the page cache on both cgroup versions, per-second rates from two
//! messages, the counters `/metrics` reports as Docker gives them, and Docker's timestamps read to
//! the nanosecond.
use super::*;
use serde_json::json;

/// A cgroup v2 message as Docker 28 sends it on the stats stream.
fn message(read: &str, cpu: u64, system: u64, disk: (u64, u64), net: (u64, u64)) -> Value {
    json!({
        "read": read,
        "preread": "2026-10-16T09:29:59.000000000Z",
        "cpu_stats": {
            "cpu_usage": {"total_usage": cpu, "usage_in_kernelmode": 0, "usage_in_usermode": cpu},
            "system_cpu_usage": system,
            "online_cpus": 8,
            "throttling_data": {"periods": 0, "throttled_periods": 0, "throttled_time": 0}
        },
        "precpu_stats": {
            "cpu_usage": {"total_usage": 1_000_000_000u64},
            "system_cpu_usage": 100_000_000_000u64,
            "online_cpus": 8
        },
        "memory_stats": {
            "usage": 900 << 20,
            "limit": 4u64 << 30,
            "stats": {"inactive_file": 100 << 20, "anon": 700 << 20}
        },
        "blkio_stats": {
            "io_service_bytes_recursive": [
                {"major": 8, "minor": 0, "op": "read", "value": disk.0},
                {"major": 8, "minor": 0, "op": "write", "value": disk.1},
                {"major": 8, "minor": 16, "op": "write", "value": 0}
            ]
        },
        "networks": {
            "eth0": {"rx_bytes": net.0, "tx_bytes": net.1},
            "eth1": {"rx_bytes": 0, "tx_bytes": 0}
        }
    })
}

#[test]
fn cpu_is_cores_of_the_host_by_the_docker_stats_formula() {
    // In 8 s of host CPU time across 8 CPUs (one second of wall time), the container used 1.5 s:
    // `docker stats` shows 150.00 %, which is 1.5 cores.
    let current = reading(&message(
        "2026-10-16T09:30:00.000000000Z",
        2_500_000_000,
        108_000_000_000,
        (0, 0),
        (0, 0),
    ))
    .unwrap();
    assert_eq!(current.online_cpus, 8);
    assert!((cores(&current) - 1.5).abs() < 1e-6, "{}", cores(&current));
    // The first message of a stream has nothing to measure against.
    let mut first = current.clone();
    first.presystem = 0;
    assert_eq!(cores(&first), 0.0);
}

#[test]
fn memory_leaves_out_the_page_cache_on_both_cgroup_versions() {
    let v2 = reading(&message("2026-10-16T09:30:00Z", 1, 1, (0, 0), (0, 0))).unwrap();
    assert_eq!(v2.memory, 800 << 20);
    assert_eq!(v2.memory_limit, 4 << 30);
    let mut v1 = message("2026-10-16T09:30:00Z", 1, 1, (0, 0), (0, 0));
    v1["memory_stats"]["stats"] = json!({"total_inactive_file": 300 << 20, "cache": 400 << 20});
    assert_eq!(reading(&v1).unwrap().memory, 600 << 20);
    // A cache figure above the usage is not trusted.
    v1["memory_stats"]["stats"] = json!({"total_inactive_file": 2u64 << 30});
    assert_eq!(reading(&v1).unwrap().memory, 900 << 20);
}

#[test]
fn disk_and_network_become_rates_between_two_messages() {
    let before = reading(&message(
        "2026-10-16T09:30:00.000000000Z",
        2_000_000_000,
        108_000_000_000,
        (1000, 5000),
        (10_000, 2000),
    ))
    .unwrap();
    let after = reading(&message(
        "2026-10-16T09:30:00.500000000Z",
        2_000_000_000,
        108_000_000_000,
        (1000, 5000 + 2048),
        (10_000 + 600, 2000 + 50),
    ))
    .unwrap();
    // Half a second apart, so each counter's change is doubled.
    let second = measure(&before, &after).unwrap();
    assert_eq!(second.disk_read, 0.0);
    assert_eq!(second.disk_write, 4096.0);
    assert_eq!(second.net_rx, 1200.0);
    assert_eq!(second.net_tx, 100.0);
    assert_eq!(second.memory, 800 << 20);
    // Messages out of order, or a repeated one, measure nothing.
    assert_eq!(measure(&after, &before), None);
    assert_eq!(measure(&after, &after), None);
    // A restarted container's counters start again from zero; that is no negative traffic.
    let restarted = reading(&message(
        "2026-10-16T09:30:01.500000000Z",
        2_000_000_000,
        108_000_000_000,
        (0, 0),
        (300, 0),
    ))
    .unwrap();
    let second = measure(&after, &restarted).unwrap();
    assert_eq!(
        (second.disk_write, second.net_rx, second.net_tx),
        (0.0, 0.0, 0.0)
    );
}

#[test]
fn block_io_operations_are_matched_in_either_case() {
    let mut value = message("2026-10-16T09:30:00Z", 1, 1, (0, 0), (0, 0));
    value["blkio_stats"]["io_service_bytes_recursive"] = json!([
        {"op": "Read", "value": 10}, {"op": "Write", "value": 20}, {"op": "Sync", "value": 30},
        {"op": "Total", "value": 30}, {"op": "", "value": 99}, {"value": 99}
    ]);
    let counters = reading(&value).unwrap();
    assert_eq!((counters.disk_read, counters.disk_write), (10, 20));
    // Docker sends null without block devices, and no networks for a container without any.
    value["blkio_stats"]["io_service_bytes_recursive"] = Value::Null;
    value.as_object_mut().unwrap().remove("networks");
    let counters = reading(&value).unwrap();
    assert_eq!((counters.disk_read, counters.net_rx), (0, 0));
}

#[test]
fn a_stopped_containers_empty_message_is_not_a_reading() {
    // Docker keeps a stream open after a stop and sends only the name and ID each second.
    let empty = json!({"read": "0001-01-01T00:00:00Z", "name": "/thoric-a", "id": "a", "cpu_stats": {"cpu_usage": {"total_usage": 0}, "throttling_data": {}}, "memory_stats": {}});
    assert_eq!(reading(&empty), None);
    let mut zero = message("2026-10-16T09:30:00Z", 0, 0, (0, 0), (0, 0));
    assert_eq!(reading(&zero), None);
    // Without online_cpus, the per-CPU list says how many there are.
    zero["cpu_stats"]["system_cpu_usage"] = json!(5);
    zero["cpu_stats"]["online_cpus"] = Value::Null;
    zero["cpu_stats"]["cpu_usage"]["percpu_usage"] = json!([1, 2, 3, 4]);
    assert_eq!(reading(&zero).unwrap().online_cpus, 4);
}

#[test]
fn docker_timestamps_are_read_to_the_nanosecond_with_their_offset() {
    let second = 1_000_000_000i128;
    let base = unix_nanos("2026-10-16T09:30:00Z").unwrap();
    assert_eq!(base, 1_792_143_000 * second);
    assert_eq!(unix_nanos("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(
        unix_nanos("2000-02-29T00:00:00Z"),
        Some(951_782_400 * second)
    );
    assert_eq!(
        unix_nanos("2026-10-16T09:30:00.123456789Z"),
        Some(base + 123_456_789)
    );
    // Fewer digits are tenths, hundredths...; more than nine are cut off.
    assert_eq!(
        unix_nanos("2026-10-16T09:30:00.5Z"),
        Some(base + 500_000_000)
    );
    assert_eq!(
        unix_nanos("2026-10-16T09:30:00.1234567891Z"),
        Some(base + 123_456_789)
    );
    // A daemon on local time writes its offset.
    assert_eq!(unix_nanos("2026-10-16T12:30:00+03:00"), Some(base));
    assert_eq!(unix_nanos("2026-10-16T04:00:00-05:30"), Some(base));
    // Go's zero time parses, far in the past.
    assert!(unix_nanos("0001-01-01T00:00:00Z").unwrap() < 0);
    for invalid in [
        "",
        "2026-10-16",
        "2026-10-16 09:30:00Z",
        "2026-13-16T09:30:00Z",
        "2026-10-16T24:30:00Z",
        "2026-10-16T09:30:00",
        "2026-10-16T09:30:00.Z",
        "2026-10-16T09:30:00+0300",
        "+026-10-16T09:30:00Z",
        "2026-10-16T09:30:00Zjunk",
    ] {
        assert_eq!(unix_nanos(invalid), None, "{invalid}");
    }
}

#[test]
fn the_counters_are_docker_s_own_since_the_container_started() {
    let mut sent = message(
        "2026-10-16T09:30:00.000000000Z",
        2_500_000_000,
        108_000_000_000,
        (4096, 8192),
        (300, 200),
    );
    sent["pids_stats"] = json!({"current": 42, "limit": 16384});
    let current = reading(&sent).unwrap();
    assert_eq!(
        counters(&current, &sent),
        Counters {
            cpu_time: 2_500_000_000,
            cpu: cores(&current),
            memory: 800 << 20,
            memory_limit: 4 << 30,
            disk_read: 4096,
            disk_write: 8192,
            net_rx: 300,
            net_tx: 200,
            pids: Some(42),
        }
    );
    // A daemon that counts no processes says nothing about them.
    sent["pids_stats"] = json!({});
    assert_eq!(counters(&current, &sent).pids, None);
}
