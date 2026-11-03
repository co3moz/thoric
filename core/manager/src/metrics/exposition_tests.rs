//! Pins down the text format Prometheus parses: a family's header comes once before its samples,
//! label values are escaped, help text keeps to one line and values are spelled as Go reads them.
use super::*;

#[test]
fn a_family_has_its_header_once_then_its_samples() {
    let mut page = Page::default();
    page.family("thoric_info", Kind::Gauge, "Thoric's version.")
        .sample(&[("version", "0.9.2")], 1.0);
    page.family("thoric_jobs", Kind::Gauge, "Jobs.")
        .sample(&[("state", "queued")], 0.0)
        .sample(&[("state", "running")], 2.0);
    page.family("thoric_jobs_finished_total", Kind::Counter, "Ended jobs.")
        .sample(&[("kind", "exec"), ("status", "failed")], 3.0);
    page.family("thoric_human_requests_pending", Kind::Gauge, "Requests.")
        .sample(&[], 1.0);
    assert_eq!(
        page.finish(),
        "# HELP thoric_info Thoric's version.\n\
         # TYPE thoric_info gauge\n\
         thoric_info{version=\"0.9.2\"} 1\n\
         # HELP thoric_jobs Jobs.\n\
         # TYPE thoric_jobs gauge\n\
         thoric_jobs{state=\"queued\"} 0\n\
         thoric_jobs{state=\"running\"} 2\n\
         # HELP thoric_jobs_finished_total Ended jobs.\n\
         # TYPE thoric_jobs_finished_total counter\n\
         thoric_jobs_finished_total{kind=\"exec\",status=\"failed\"} 3\n\
         # HELP thoric_human_requests_pending Requests.\n\
         # TYPE thoric_human_requests_pending gauge\n\
         thoric_human_requests_pending 1\n"
    );
}

#[test]
fn label_values_are_escaped_and_keep_no_control_characters() {
    assert_eq!(
        escape_label("C:\\Users \"dev\"\nbox\r\t\u{7}"),
        "C:\\\\Users \\\"dev\\\"\\nbox"
    );
    assert_eq!(escape_label("Masaüstü ünlü"), "Masaüstü ünlü");
    let mut page = Page::default();
    page.family(
        "thoric_container_pids",
        Kind::Gauge,
        "Line one\nand a \\ two.",
    )
    .sample(&[("id", "a"), ("name", "x\"} 1\nforged 2")], 3.0);
    let text = page.finish();
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        [
            "# HELP thoric_container_pids Line one\\nand a \\\\ two.",
            "# TYPE thoric_container_pids gauge",
            "thoric_container_pids{id=\"a\",name=\"x\\\"} 1\\nforged 2\"} 3",
        ]
    );
}

#[test]
fn values_are_spelled_as_go_reads_them() {
    assert_eq!(number(0.0), "0");
    assert_eq!(number(1_792_141_200.5), "1792141200.5");
    assert_eq!(number(17_179_869_184.0), "17179869184");
    assert_eq!(number(0.125), "0.125");
    assert_eq!(number(f64::NAN), "NaN");
    assert_eq!(number(f64::INFINITY), "+Inf");
    assert_eq!(number(f64::NEG_INFINITY), "-Inf");
}
