//! Prometheus' text exposition format, version 0.0.4, written by hand: each family's `# HELP` and
//! `# TYPE` once, then its samples, one line each, with the label values escaped.
use std::fmt::Write;

/// The media type of the format.
pub(crate) const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Counter,
    Gauge,
}

/// The page being written.
#[derive(Debug, Default)]
pub(crate) struct Page(String);

/// A family whose samples follow its header. It borrows the page, so the next family can only
/// begin once this one is done and a family's samples stay together, as the format asks.
pub(crate) struct Family<'a> {
    page: &'a mut String,
    name: &'static str,
}

impl Page {
    /// Begin the family `name`, of names the code chooses, which Prometheus' rules allow.
    pub(crate) fn family(&mut self, name: &'static str, kind: Kind, help: &str) -> Family<'_> {
        debug_assert!(valid_name(name), "{name}");
        let kind = match kind {
            Kind::Counter => "counter",
            Kind::Gauge => "gauge",
        };
        let _ = writeln!(self.0, "# HELP {name} {}", escape_help(help));
        let _ = writeln!(self.0, "# TYPE {name} {kind}");
        Family {
            page: &mut self.0,
            name,
        }
    }

    pub(crate) fn finish(self) -> String {
        self.0
    }
}

impl Family<'_> {
    /// One sample, with labels of names the code chooses and values of any text.
    pub(crate) fn sample(&mut self, labels: &[(&str, &str)], value: f64) -> &mut Self {
        self.page.push_str(self.name);
        if !labels.is_empty() {
            self.page.push('{');
            for (index, (name, label)) in labels.iter().enumerate() {
                debug_assert!(valid_name(name) && !name.contains(':'), "{name}");
                if index > 0 {
                    self.page.push(',');
                }
                let _ = write!(self.page, "{name}=\"{}\"", escape_label(label));
            }
            self.page.push('}');
        }
        let _ = writeln!(self.page, " {}", number(value));
        self
    }
}

/// A metric or label name: `[a-zA-Z_:][a-zA-Z0-9_:]*`.
fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || b"_:".contains(&first))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || b"_:".contains(&byte))
}

/// A label value with its backslashes, quotes and line feeds escaped, as the format asks, and
/// without the other control characters, which some parsers misread.
pub(crate) fn escape_label(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            other if other.is_control() => {}
            other => escaped.push(other),
        }
    }
    escaped
}

/// A help text with its backslashes and line feeds escaped.
fn escape_help(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

/// A value as Go's `ParseFloat` reads it, which spells the infinities `+Inf` and `-Inf`.
pub(crate) fn number(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value == f64::INFINITY {
        "+Inf".into()
    } else if value == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
#[path = "exposition_tests.rs"]
mod tests;
