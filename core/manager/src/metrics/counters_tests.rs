//! Pins down that the counters keep their labels bounded: a tool outside Thoric's catalog is
//! counted as unknown whatever it was called, desktop servers' tools are counted together, and
//! jobs by kind and status.
use super::*;

#[test]
fn a_tool_is_counted_by_its_name_only_when_the_catalog_has_it() {
    let metrics = Metrics::default();
    metrics.called(Source::Tool, "list_containers", Outcome::Ok);
    metrics.called(Source::Tool, "list_containers", Outcome::Ok);
    metrics.called(Source::Tool, "list_containers", Outcome::Error);
    for invented in ["no_such_tool", "x\"} 1\nforged", ""] {
        metrics.called(Source::Tool, invented, Outcome::Refused);
    }
    metrics.called(Source::Server, "browser_click", Outcome::Ok);
    metrics.called(Source::Server, "anything-a-desktop-declares", Outcome::Ok);
    metrics.called(Source::Transfer, "container_upload", Outcome::Ok);
    let counts = metrics.counts();
    assert_eq!(
        counts.tools,
        BTreeMap::from([
            (("list_containers".to_owned(), "error"), 1),
            (("list_containers".to_owned(), "ok"), 2),
            ((UNKNOWN_TOOL.to_owned(), "refused"), 3),
        ])
    );
    assert_eq!(counts.servers, BTreeMap::from([("ok", 2)]));
    assert_eq!(
        counts.transfers,
        BTreeMap::from([(("container_upload".to_owned(), "ok"), 1)])
    );
}

#[test]
fn jobs_are_counted_by_kind_and_status() {
    let metrics = Metrics::default();
    metrics.job_ended("exec", "completed");
    metrics.job_ended("exec", "failed");
    metrics.job_ended("exec", "completed");
    metrics.job_ended("build", "failed");
    assert_eq!(
        metrics.counts().jobs,
        BTreeMap::from([
            (("build".to_owned(), "failed".to_owned()), 1),
            (("exec".to_owned(), "completed".to_owned()), 2),
            (("exec".to_owned(), "failed".to_owned()), 1),
        ])
    );
}

#[tokio::test]
async fn the_volume_count_is_unknown_until_docker_answers() {
    let app = crate::test_support::app(false);
    assert_eq!(app.metrics.volumes(&app), None);
    // The test installation's Docker command does not exist, so the count stays unknown.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(app.metrics.volumes(&app), None);
}
