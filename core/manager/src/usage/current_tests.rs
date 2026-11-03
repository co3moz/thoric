//! Pins down what `/metrics` is told of the collector: a container recreated under a new Docker ID
//! starts again with nothing known, a stream of the container it replaced writes nothing, a failed
//! listing forgets every state but not what the streams learnt, and the host's GPU is read only
//! while it is sampled.
use super::*;

fn running(entries: &[(&str, &str)]) -> BTreeMap<String, engine::Running> {
    entries
        .iter()
        .map(|(id, docker_id)| {
            let running = engine::Running {
                docker_id: (*docker_id).into(),
                gpu: false,
            };
            ((*id).to_owned(), running)
        })
        .collect()
}

fn counters(cpu_time: u64) -> Counters {
    Counters {
        cpu_time,
        ..Default::default()
    }
}

#[test]
fn a_recreated_container_starts_again_and_its_predecessor_writes_nothing() {
    let usage = Usage::default();
    let states = BTreeMap::from([("a".to_owned(), "running".to_owned())]);
    usage.listed(states.clone(), &running(&[("a", "1")]));
    usage.started("a", "1", 100.0);
    usage.counted("a", "1", counters(5));
    let now = usage.current();
    assert_eq!(now.states, Some(states.clone()));
    assert_eq!(
        now.running,
        [Running {
            id: "a".into(),
            started: Some(100.0),
            counters: Some(counters(5)),
        }]
    );
    usage.listed(states, &running(&[("a", "2")]));
    // The stream of the container that was replaced is still being read.
    usage.counted("a", "1", counters(9));
    usage.started("a", "1", 200.0);
    assert_eq!(
        usage.current().running,
        [Running {
            id: "a".into(),
            started: None,
            counters: None,
        }]
    );
    usage.counted("a", "2", counters(1));
    assert_eq!(usage.current().running[0].counters, Some(counters(1)));
}

#[test]
fn a_failed_listing_forgets_the_states_and_keeps_what_the_streams_learnt() {
    let usage = Usage::default();
    assert_eq!(usage.current().states, None);
    let states = BTreeMap::from([("a".to_owned(), "running".to_owned())]);
    usage.listed(states.clone(), &running(&[("a", "1")]));
    usage.started("a", "1", 100.0);
    usage.unlisted();
    assert_eq!(usage.current().states, None);
    // The stream goes on and opens no inspect again, so its start has to outlast the gap.
    usage.listed(states, &running(&[("a", "1")]));
    assert_eq!(usage.current().running[0].started, Some(100.0));
}

#[test]
fn the_host_gpu_is_read_only_while_it_is_sampled() {
    let usage = Usage::default();
    assert_eq!(usage.current().gpu_state, "idle");
    crate::test_support::sampled_gpu(&usage, 0, "NVIDIA RTX", 40.0);
    let now = usage.current();
    assert_eq!(now.gpu_state, "sampling");
    assert_eq!(
        now.gpus,
        [Gpu {
            index: 0,
            name: "NVIDIA RTX".into(),
            utilization: Some(40.0),
            memory_used: Some(1 << 30),
            memory_total: Some(8 << 30),
        }]
    );
    // The GPU desktop stopped: its samples stay for the charts, and are no reading now.
    usage.set_gpu(GpuState::Idle, None);
    let now = usage.current();
    assert_eq!(now.gpu_state, "idle");
    assert!(now.gpus.is_empty());
}
