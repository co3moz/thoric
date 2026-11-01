//! Pins down how a script build reports where it is: stages only move forward and close the one
//! before them, BuildKit's own progress lines start the layer and image stages while the script's
//! output never does, and the log names each stage on a line of its own. A snapshot and a settings
//! change that recreates its container report their own stages the same way, and a stage name
//! means one step in every job.
use super::*;
use crate::model::Job;

fn job(stages: bool) -> Job {
    let mut job = Job {
        id: "build".into(),
        kind: "build".into(),
        resource: None,
        status: "running".into(),
        created_at: 1,
        finished_at: None,
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    };
    if stages {
        job.stages = BUILD_STAGES
            .iter()
            .map(|(name, _)| Stage {
                name: (*name).into(),
                started_at: None,
                finished_at: None,
            })
            .collect();
    }
    job
}

fn times(job: &Job) -> Vec<(&str, Option<u64>, Option<u64>)> {
    job.stages
        .iter()
        .map(|stage| (stage.name.as_str(), stage.started_at, stage.finished_at))
        .collect()
}

#[test]
fn stages_move_forward_and_finish_the_running_one() {
    let mut job = job(true);
    assert_eq!(job.begin_stage("base", 10), Some(0));
    assert_eq!(job.begin_stage("script", 12), Some(1));
    // A repeated marker, or one for a stage already passed, changes nothing.
    assert_eq!(job.begin_stage("script", 13), None);
    assert_eq!(job.begin_stage("base", 14), None);
    assert_eq!(job.begin_stage("unknown", 14), None);
    // Without a marker for it, the layer stage is skipped rather than given made-up times.
    assert_eq!(job.begin_stage("image", 20), Some(3));
    assert_eq!(job.begin_stage("layer", 21), None);
    job.finish_stages(25);
    assert_eq!(
        times(&job),
        [
            ("base", Some(10), Some(12)),
            ("script", Some(12), Some(20)),
            ("layer", None, None),
            ("image", Some(20), Some(25)),
            ("cleanup", None, None),
        ]
    );
    // A stage that never started carries only its name.
    let value = serde_json::to_value(&job).unwrap();
    assert_eq!(value["stages"][4], serde_json::json!({"name": "cleanup"}));
    assert_eq!(
        value["stages"][0],
        serde_json::json!({"name": "base", "started_at": 10, "finished_at": 12})
    );
    // Other jobs keep their shape.
    assert!(serde_json::to_value(self::job(false)).unwrap()["stages"].is_null());
}

/// `docker build --progress=plain` output as BuildKit writes it, with the classic image store
/// and then with the containerd image store.
const CLASSIC: &str = "#0 building with \"default\" instance using docker driver

#1 [internal] load build definition from Dockerfile
#1 transferring dockerfile: 247B done
#1 DONE 0.0s

#4 [1/3] FROM docker.io/thoric/build-base:0f3a
#4 DONE 0.1s

#6 [2/3] COPY setup.sh /tmp/thoric-setup.sh
#6 DONE 0.1s

#7 [3/3] RUN --mount=type=bind,source=thoric_trace.sh,target=/tmp/thoric_trace.sh BASH_ENV=/tmp/thoric_trace.sh bash /tmp/thoric-setup.sh && rm /tmp/thoric-setup.sh
#7 0.231 exporting to image
#7 0.232 #9 naming to nowhere
#7 2.004 Setting up python3 (3.12.3-0ubuntu2) ...
#7 DONE 250.2s

#8 exporting to image
#8 exporting layers
#8 exporting layers 171.4s done
#8 writing image sha256:5f1d2c done
#8 naming to docker.io/thoric/custom:0f3a done
#8 DONE 171.6s
";
const CONTAINERD: &str = "#8 exporting to image
#8 exporting layers 12.3s done
#8 exporting manifest sha256:aa done
#8 exporting config sha256:bb done
#8 exporting attestation manifest sha256:cc done
#8 exporting manifest list sha256:dd done
#8 naming to docker.io/thoric/custom:0f3a done
#8 unpacking to docker.io/thoric/custom:0f3a 4.1s done
#8 DONE 16.5s
";

#[test]
fn buildkit_progress_lines_start_the_layer_and_image_stages() {
    let marked = |text: &str| {
        text.lines()
            .filter_map(|line| buildkit_stage(line).map(|stage| (line.to_owned(), stage)))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        marked(CLASSIC),
        [
            ("#8 exporting to image".into(), "layer"),
            ("#8 exporting layers".into(), "layer"),
            ("#8 exporting layers 171.4s done".into(), "layer"),
            ("#8 writing image sha256:5f1d2c done".into(), "image"),
            (
                "#8 naming to docker.io/thoric/custom:0f3a done".into(),
                "image"
            ),
        ]
    );
    let stages = marked(CONTAINERD)
        .into_iter()
        .map(|(_, stage)| stage)
        .collect::<Vec<_>>();
    assert_eq!(
        stages,
        [
            "layer", "layer", "image", "image", "image", "image", "image", "image"
        ]
    );
    // The legacy builder's steps and anything that is not a step line start nothing.
    for line in [
        "Step 4/6 : RUN bash /tmp/thoric-setup.sh && rm /tmp/thoric-setup.sh",
        "exporting to image",
        "# exporting to image",
        "#x exporting to image",
        "Stage 3/5: Saving the image layer",
    ] {
        assert_eq!(buildkit_stage(line), None, "{line}");
    }
}

#[test]
fn lines_wait_for_their_end() {
    let mut lines = Lines::default();
    assert_eq!(lines.push("#8 export"), "");
    assert_eq!(
        lines.push("ing to image\n#8 exp"),
        "#8 exporting to image\n"
    );
    assert_eq!(lines.push(""), "");
    assert_eq!(lines.finish(), "#8 exp");
    assert_eq!(lines.finish(), "");
    // A line that never ends is logged once it is too long to be a marker.
    let long = "x".repeat(LINE_LIMIT + 1);
    assert_eq!(lines.push(&long), long);
}

#[test]
fn build_output_names_each_stage_on_its_own_log_line() {
    let app = crate::test_support::app(false);
    app.db.put("job", "build", &job(false)).unwrap();
    app.plan_stages("build", &BUILD_STAGES).unwrap();
    app.begin_stage("build", "base").unwrap();
    let download = serde_json::json!({"current": 1, "total": 2, "layers": 1, "layers_done": 0});
    app.set_progress("build", Some(download)).unwrap();
    app.begin_stage("build", "script").unwrap();
    // Output arrives in pieces that split lines, the marker line included.
    let mut progress = ScriptProgress::build(vec![crate::script_progress::ScriptPart {
        id: "setup".into(),
        name: "Staged".into(),
        script: "true\n".into(),
    }]);
    let mut lines = Lines::default();
    let mut rest = CLASSIC;
    while !rest.is_empty() {
        let (piece, after) = rest.split_at(rest.len().min(37));
        app.log_script_output("build", &lines.push(piece), &mut progress)
            .unwrap();
        rest = after;
    }
    app.log_script_output("build", &lines.finish(), &mut progress)
        .unwrap();
    let job: Job = app.db.job("build").unwrap();
    assert_eq!(
        begun(&job),
        [
            ("base", true),
            ("script", true),
            ("layer", true),
            ("image", true),
            ("cleanup", false)
        ]
    );
    // The download's progress ended with the stage it belonged to, and the script's with the
    // layer stage; the script's one part ran from its step's first line to its end.
    assert_eq!(job.progress, None);
    let part = &job.script.as_ref().unwrap().parts[0];
    assert!(
        part.started_at.is_some() && part.finished_at.is_some(),
        "{part:?}"
    );
    let log = job.log.lines().collect::<Vec<_>>();
    assert_eq!(log[0], "Stage 1/5: Preparing the base image");
    assert_eq!(log[1], "Stage 2/5: Running the setup script");
    let layer = log
        .iter()
        .position(|line| *line == "Stage 3/5: Saving the image layer")
        .unwrap();
    assert_eq!(log[layer + 1], "#8 exporting to image");
    let image = log
        .iter()
        .position(|line| *line == "Stage 4/5: Writing and tagging the image")
        .unwrap();
    assert_eq!(log[image + 1], "#8 writing image sha256:5f1d2c done");
    // Every line BuildKit wrote is in the log, whole and in order.
    let output = log
        .iter()
        .filter(|line| !line.starts_with("Stage "))
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(output, CLASSIC.lines().collect::<Vec<_>>());
}

/// The dashboard names each stage from its catalog and the guides list them, so a stage added here
/// without a name there would show as its bare identifier.
#[test]
fn every_job_stage_is_named_in_the_interface_and_documented() {
    let catalog = include_str!("../../web/src/locales/en.ts");
    for (name, _) in JOB_STAGES.iter().flat_map(|stages| stages.iter()) {
        assert!(
            catalog.contains(&format!("\"stage.{name}\":")),
            "core/web/src/locales/en.ts must name the stage {name}"
        );
        for (path, guide) in [
            ("docs/API.md", include_str!("../../../docs/API.md")),
            ("docs/MCP.md", include_str!("../../../docs/MCP.md")),
        ] {
            assert!(
                guide.contains(&format!("`{name}`")),
                "{path} must document the stage {name}"
            );
        }
    }
}

/// The interface has one label per stage name, so a name shared by two jobs must be the same
/// step in both, and a job cannot list a stage twice.
#[test]
fn a_stage_name_means_one_step_in_every_job() {
    let mut labels = std::collections::HashMap::new();
    for stages in JOB_STAGES {
        let mut names = std::collections::HashSet::new();
        for (name, line) in stages {
            assert!(names.insert(name), "{name} is listed twice in one job");
            let first = *labels.entry(name).or_insert(line);
            assert_eq!(first, line, "{name} has two labels");
        }
    }
    assert_eq!(label("commit"), "Committing the container layer");
    assert_eq!(label("unknown"), "unknown");
}

fn begun(job: &Job) -> Vec<(&str, bool)> {
    job.stages
        .iter()
        .map(|stage| (stage.name.as_str(), stage.started_at.is_some()))
        .collect()
}

/// A manager whose Docker CLI is a shell script in a new directory, which it enters before
/// `commands` run with the CLI's arguments.
#[cfg(unix)]
fn fake_docker(commands: &str) -> (std::sync::Arc<App>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("thoric-stages-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        &format!("#!/bin/sh\ncd '{}'\n{commands}", dir.display()),
    );
    let mut app = crate::test_support::app(false);
    let config = &mut std::sync::Arc::get_mut(&mut app).unwrap().config;
    config.docker_bin = docker.display().to_string();
    config.data = dir.join("data");
    (app, dir)
}

#[cfg(unix)]
async fn finished(app: &App, started: axum::Json<serde_json::Value>) -> Job {
    let id = started.0["job_id"].as_str().unwrap();
    loop {
        let job: Job = app.db.job(id).unwrap();
        if job.finished_at.is_some() {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// The container `e` as the manager records it, running in Docker as `thoric-e`.
#[cfg(unix)]
fn container(app: &App) {
    let env = crate::model::Container {
        id: "e".into(),
        container: "thoric-e".into(),
        private_network: "thoric-e-private".into(),
        egress_network: "thoric-e-egress".into(),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec: serde_json::from_value(
            serde_json::json!({"name": "Work", "image": "example/desktop:1"}),
        )
        .unwrap(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    app.db.put(crate::model::CONTAINERS, "e", &env).unwrap();
}

#[cfg(unix)]
const CONTAINER_JSON: &str = r#"{"Id":"c0ffee","Config":{"Labels":{"thoric.managed":"true","thoric.environment":"e"}},"State":{"Running":RUNNING}}"#;

#[cfg(unix)]
#[tokio::test]
async fn a_build_job_reports_its_stages_and_where_it_failed() {
    // Images are desktop images, and a build writes the prepared output.
    let (app, dir) = fake_docker(
        r#"case "$1" in
  build) cp "$7/Dockerfile" "$7/thoric_trace.sh" .; cat build-output >&2; exit $(cat build-status) ;;
esac
case "$1 $2" in
  'image inspect') echo '[{"Id":"sha256:feed","Config":{"Labels":{"thoric.desktop":"true"}}}]' ;;
esac
"#,
    );
    let run = |output: &str, status: u8| {
        std::fs::write(dir.join("build-output"), output).unwrap();
        std::fs::write(dir.join("build-status"), status.to_string()).unwrap();
        let app = app.clone();
        async move {
            let body = crate::api::BuildImage {
                name: "Staged".into(),
                description: String::new(),
                base_image: "example/desktop:1".into(),
                internet: false,
                script: "true\n".into(),
                compose: None,
                inherit_compose: true,
                icon: None,
                template_id: None,
                template_ids: Vec::new(),
                parts: Vec::new(),
                mcp: Vec::new(),
            };
            let started = crate::api::build(axum::extract::State(app.clone()), axum::Json(body))
                .await
                .unwrap();
            finished(&app, started).await
        }
    };

    let built = run(CLASSIC, 0).await;
    assert_eq!(built.status, "completed", "{}", built.log);
    for stage in &built.stages {
        let (Some(started), Some(finished)) = (stage.started_at, stage.finished_at) else {
            panic!("{} did not run: {:?}", stage.name, built.stages);
        };
        assert!(started <= finished && finished <= built.finished_at.unwrap());
    }
    assert!(built.log.contains("Stage 5/5: Cleaning up\n"));
    // Bash reads the line trace before the script, from a mount that leaves nothing in the image.
    let dockerfile = std::fs::read_to_string(dir.join("Dockerfile")).unwrap();
    assert!(dockerfile.contains("COPY setup.sh /tmp/thoric-setup.sh\nRUN --mount=type=bind,source=thoric_trace.sh,target=/tmp/thoric_trace.sh BASH_ENV=/tmp/thoric_trace.sh bash /tmp/thoric-setup.sh && rm /tmp/thoric-setup.sh\n"), "{dockerfile}");
    assert_eq!(
        std::fs::read_to_string(dir.join("thoric_trace.sh")).unwrap(),
        crate::script_progress::TRACE
    );
    assert!(
        built.script.as_ref().unwrap().parts[0]
            .finished_at
            .is_some()
    );

    // A failed script leaves its own stage as the last one started, and its part without an end;
    // cleanup still runs unlisted.
    let failed = run(
        "#7 [3/3] RUN BASH_ENV=/tmp/thoric_trace.sh bash /tmp/thoric-setup.sh && rm /tmp/thoric-setup.sh\n#7 0.1 THORIC_LINE=1\n#7 0.1 nope\n#7 ERROR: exit code: 17\n",
        1,
    )
    .await;
    let part = &failed.script.as_ref().unwrap().parts[0];
    assert!(
        part.started_at.is_some() && part.finished_at.is_none(),
        "{part:?}"
    );
    assert!(failed.log.contains("#7 0.1 nope\n") && !failed.log.contains("THORIC_LINE"));
    assert_eq!(failed.status, "failed");
    assert_eq!(
        begun(&failed),
        [
            ("base", true),
            ("script", true),
            ("layer", false),
            ("image", false),
            ("cleanup", false)
        ]
    );
    assert!(failed.stages[1].finished_at.is_some());
    assert!(!dir.join("data/builds").join(&failed.id).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_snapshot_reports_its_stages_and_where_it_failed() {
    let (app, dir) = fake_docker(
        r#"case "$1 $2" in
  'container inspect') printf '[%s]' "$(cat container.json)" ;;
  'image inspect') echo '[{"Id":"sha256:snap","Config":{"Labels":{}}}]' ;;
esac
case "$1" in
  commit) exit $(cat commit-status) ;;
esac
"#,
    );
    container(&app);
    std::fs::write(
        dir.join("container.json"),
        CONTAINER_JSON.replace("RUNNING", "true"),
    )
    .unwrap();
    let run = |status: u8| {
        std::fs::write(dir.join("commit-status"), status.to_string()).unwrap();
        let app = app.clone();
        async move {
            let body = crate::api::Snapshot {
                name: "Staged".into(),
                description: String::new(),
            };
            let started = crate::api::snapshot(
                axum::extract::State(app.clone()),
                axum::extract::Path("e".into()),
                axum::Json(body),
            )
            .await
            .unwrap();
            finished(&app, started).await
        }
    };

    let saved = run(0).await;
    assert_eq!(saved.status, "completed", "{}", saved.log);
    assert_eq!(
        begun(&saved),
        [("check", true), ("commit", true), ("record", true)]
    );
    // Each stage ends where the next begins, and the last one with the job.
    assert_eq!(saved.stages[0].finished_at, saved.stages[1].started_at);
    assert_eq!(saved.stages[1].finished_at, saved.stages[2].started_at);
    assert_eq!(saved.stages[2].finished_at, saved.finished_at);
    let log = saved.log.lines().collect::<Vec<_>>();
    assert_eq!(log[0], "Stage 1/3: Checking the container");
    assert_eq!(log[1], "Stage 2/3: Committing the container layer");
    assert!(
        log[2].starts_with("Committing container changes."),
        "{log:?}"
    );
    assert!(saved.log.contains("Stage 3/3: Saving the image record\n"));

    // A commit Docker refused is where the snapshot stopped, and no image record follows it.
    let failed = run(1).await;
    assert_eq!(failed.status, "failed");
    assert_eq!(
        begun(&failed),
        [("check", true), ("commit", true), ("record", false)]
    );
    assert_eq!(failed.stages[1].finished_at, failed.finished_at);
    assert!(
        app.db
            .get::<crate::model::ImageRecord>("image", &failed.id)
            .is_err()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_recreating_settings_change_reports_its_stages_and_where_it_failed() {
    // Only the container itself exists, so the name kept for its predecessor is free.
    let (app, dir) = fake_docker(
        r#"echo "$*" >> calls
case "$1 $2 $3" in
  'container inspect thoric-e') printf '[%s]' "$(cat container.json)"; exit ;;
  'container inspect '*) exit 1 ;;
  'image inspect '*) echo '[{"Id":"sha256:work","Config":{}}]'; exit ;;
  'network inspect '*) echo '[{"IPAM":{"Config":[{"Subnet":"172.30.0.0/16"}]}}]'; exit ;;
esac
case "$1" in
  create) exit $(cat create-status) ;;
esac
"#,
    );
    container(&app);
    let run = |shm_mb: u64, running: bool, status: u8| {
        std::fs::write(
            dir.join("container.json"),
            CONTAINER_JSON.replace("RUNNING", &running.to_string()),
        )
        .unwrap();
        std::fs::write(dir.join("create-status"), status.to_string()).unwrap();
        std::fs::write(dir.join("calls"), "").unwrap();
        let app = app.clone();
        async move {
            let started = crate::api::update_settings(
                axum::extract::State(app.clone()),
                axum::extract::Path("e".into()),
                axum::Json(serde_json::json!({"shm_mb": shm_mb})),
            )
            .await
            .unwrap();
            finished(&app, started).await
        }
    };
    let calls = || std::fs::read_to_string(dir.join("calls")).unwrap();

    let recreated = run(1024, true, 0).await;
    assert_eq!(recreated.status, "completed", "{}", recreated.log);
    assert_eq!(
        begun(&recreated),
        [
            ("stop", true),
            ("commit", true),
            ("create", true),
            ("start", true),
            ("cleanup", true)
        ]
    );
    for pair in recreated.stages.windows(2) {
        assert_eq!(
            pair[0].finished_at, pair[1].started_at,
            "{:?}",
            recreated.stages
        );
    }
    assert_eq!(recreated.stages[4].finished_at, recreated.finished_at);
    let log = recreated.log.lines().collect::<Vec<_>>();
    assert_eq!(log[0], "Stage 1/5: Stopping the container");
    assert_eq!(log[1], "Stopping the container. Open applications close.");
    assert!(
        recreated
            .log
            .contains("Stage 2/5: Committing the container layer\n")
    );
    assert!(
        recreated
            .log
            .contains("Stage 3/5: Creating the new container\n")
    );
    assert!(
        recreated
            .log
            .contains("Stage 4/5: Starting the new container\n")
    );
    assert!(recreated.log.contains("Stage 5/5: Cleaning up\n"));
    // The new container has started once its desktop is ready.
    assert!(
        calls().contains("exec --user 1001:1001 c0ffee test -e /run/desktop/ready\n"),
        "{}",
        calls()
    );
    // The new container is told its private network's subnet, which its VNC server listens in.
    assert!(
        calls().lines().any(|call| call.starts_with("create ")
            && call.contains("--env DESKTOP_PRIVATE_SUBNET=172.30.0.0/16 ")),
        "{}",
        calls()
    );

    // A stopped container has nothing to stop, and its replacement is not started, so those
    // stages are skipped.
    let stopped = run(2048, false, 0).await;
    assert_eq!(stopped.status, "completed", "{}", stopped.log);
    assert_eq!(
        begun(&stopped),
        [
            ("stop", false),
            ("commit", true),
            ("create", true),
            ("start", false),
            ("cleanup", true)
        ]
    );
    assert!(
        !calls().lines().any(|call| call.starts_with("stop ")),
        "{}",
        calls()
    );
    assert!(
        stopped
            .log
            .starts_with("Stage 2/5: Committing the container layer\n")
    );

    // A new container Docker refused is where the change stopped; the previous one is back.
    let failed = run(768, true, 1).await;
    assert_eq!(failed.status, "failed");
    assert_eq!(
        begun(&failed),
        [
            ("stop", true),
            ("commit", true),
            ("create", true),
            ("start", false),
            ("cleanup", false)
        ]
    );
    assert_eq!(failed.stages[2].finished_at, failed.finished_at);
    assert!(calls().contains("rename c0ffee thoric-e\n"), "{}", calls());

    // A change that applies in place lists no stages.
    let live = finished(
        &app,
        crate::api::update_settings(
            axum::extract::State(app.clone()),
            axum::extract::Path("e".into()),
            axum::Json(serde_json::json!({"name": "Renamed"})),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(live.status, "completed", "{}", live.log);
    assert!(live.stages.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}
