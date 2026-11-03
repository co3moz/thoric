//! Named stages of a long job, so the dashboard and agents can tell where a script build, a
//! snapshot or a settings change that recreates a container is and how long each part of it took.
//!
//! A job that reports stages lists every one of them once its work starts, in order, and each
//! records when it started and finished, in seconds like the job's own times. Stages only move
//! forward. One that never started was skipped, and the last one that started in a failed or
//! interrupted job is where it stopped. A stage has no percentage of its own: the job's `progress`
//! stays the progress of the running stage, which a base image download measures and the scripts
//! report themselves (`script_progress`). BuildKit reports nothing countable while it saves the
//! image layer, and `docker commit` reports nothing while it saves a container layer, so those
//! stages show only their time.
use crate::{
    App,
    model::{Job, now},
    script_progress::ScriptProgress,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// The stages of a script build, with the line each writes to the job log.
pub(crate) const BUILD_STAGES: [(&str, &str); 5] = [
    ("base", "Preparing the base image"),
    ("script", "Running the setup script"),
    ("layer", "Saving the image layer"),
    ("image", "Writing and tagging the image"),
    ("cleanup", "Cleaning up"),
];

/// The stages of a snapshot.
pub(crate) const SNAPSHOT_STAGES: [(&str, &str); 3] = [
    ("check", "Checking the container"),
    ("commit", "Committing the container layer"),
    ("record", "Saving the image record"),
];

/// The stages of a settings change that recreates the container from a commit of its files. A
/// stopped container skips `stop` and `start`, which waits for the new container's desktop.
pub(crate) const RECREATE_STAGES: [(&str, &str); 5] = [
    ("stop", "Stopping the container"),
    ("commit", "Committing the container layer"),
    ("create", "Creating the new container"),
    ("start", "Starting the new container"),
    ("cleanup", "Cleaning up"),
];

/// The stages of an installation backup. A part that is not in the backup skips its stage.
pub(crate) const BACKUP_STAGES: [(&str, &str); 4] = [
    ("plan", "Reading the installation"),
    ("images", "Transferring the saved images"),
    ("files", "Transferring the containers' files"),
    ("volumes", "Transferring the volumes"),
];

/// The stages of a restore. The upload goes on through `volumes`; the last two follow it.
pub(crate) const RESTORE_STAGES: [(&str, &str); 6] = [
    ("verify", "Checking the backup against this installation"),
    ("images", "Transferring the saved images"),
    ("files", "Transferring the containers' files"),
    ("volumes", "Transferring the volumes"),
    ("records", "Saving the records"),
    ("containers", "Creating the containers"),
];

/// The stages of a duplicate: the snapshot's, then the copies of the volumes created for the
/// original, which a container without any skips, and the new container's creation and, when it
/// is started, the wait for its desktop.
pub(crate) const DUPLICATE_STAGES: [(&str, &str); 6] = [
    ("check", "Checking the container"),
    ("commit", "Committing the container layer"),
    ("record", "Saving the image record"),
    ("volumes", "Transferring the volumes"),
    ("create", "Creating the new container"),
    ("start", "Starting the new container"),
];

/// Every job's stages. The interface names a stage by its name alone, so a name two jobs share is
/// the same step with the same words.
pub(crate) const JOB_STAGES: [&[(&str, &str)]; 6] = [
    &BUILD_STAGES,
    &SNAPSHOT_STAGES,
    &RECREATE_STAGES,
    &BACKUP_STAGES,
    &RESTORE_STAGES,
    &DUPLICATE_STAGES,
];

/// A longer line is no stage marker, so it is logged without waiting for its end.
const LINE_LIMIT: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Stage {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
}

impl Job {
    /// Start the stage `name` and finish the one that was running. A stage that already started,
    /// or one before it, is left alone, so a repeated or late marker changes nothing. Returns the
    /// started stage's position.
    pub fn begin_stage(&mut self, name: &str, at: u64) -> Option<usize> {
        let index = self.stages.iter().position(|stage| stage.name == name)?;
        if self.stages[index..]
            .iter()
            .any(|stage| stage.started_at.is_some())
        {
            return None;
        }
        self.finish_stages(at);
        self.stages[index].started_at = Some(at);
        Some(index)
    }

    /// Finish the running stage, once the job moves on or ends.
    pub fn finish_stages(&mut self, at: u64) {
        for stage in &mut self.stages {
            if stage.started_at.is_some() && stage.finished_at.is_none() {
                stage.finished_at = Some(at);
            }
        }
    }
}

/// The build stage a line of BuildKit's plain progress output begins. Exporting to the image
/// starts with saving the layer the script wrote; writing the image's configuration, naming it
/// and, with the containerd image store, unpacking it follow. Output of the script itself carries
/// its time after the step number, so it never matches.
pub(crate) fn buildkit_stage(line: &str) -> Option<&'static str> {
    let (step, rest) = line.strip_prefix('#')?.split_once(' ')?;
    if step.is_empty() || !step.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if rest.starts_with("exporting to image") || rest.starts_with("exporting layers") {
        return Some("layer");
    }
    [
        "writing image",
        "exporting manifest",
        "exporting config",
        "exporting attestation",
        "naming to",
        "unpacking to",
    ]
    .iter()
    .any(|prefix| rest.starts_with(prefix))
    .then_some("image")
}

fn label(name: &str) -> &str {
    JOB_STAGES
        .iter()
        .flat_map(|stages| stages.iter())
        .find(|(stage, _)| *stage == name)
        .map_or(name, |(_, label)| label)
}

/// Holds back the end of a stream's text until its line is complete, so a stage starts between
/// two lines of the log.
#[derive(Default)]
pub(crate) struct Lines {
    pending: String,
}
impl Lines {
    /// The whole lines available after `text`, with their line ends.
    pub(crate) fn push(&mut self, text: &str) -> String {
        self.pending.push_str(text);
        let end = match self.pending.rfind('\n') {
            Some(at) => at + 1,
            None if self.pending.len() > LINE_LIMIT => self.pending.len(),
            None => 0,
        };
        self.pending.drain(..end).collect()
    }
    /// What is left once the stream ends.
    pub(crate) fn finish(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

impl App {
    /// Give the job its stages, none of them started yet.
    pub fn plan_stages(&self, id: &str, stages: &[(&str, &str)]) -> Result<()> {
        let mut job: Job = self.db.get("job", id)?;
        job.stages = stages
            .iter()
            .map(|(name, _)| Stage {
                name: (*name).into(),
                started_at: None,
                finished_at: None,
            })
            .collect();
        self.db.put("job", id, &job)
    }

    /// Start a stage and say so in the log. The previous stage's measured progress ends with it.
    pub fn begin_stage(&self, id: &str, name: &str) -> Result<()> {
        let mut job: Job = self.db.get("job", id)?;
        let Some(index) = job.begin_stage(name, now()) else {
            return Ok(());
        };
        job.progress = None;
        let line = format!(
            "Stage {}/{}: {}\n",
            index + 1,
            job.stages.len(),
            label(name)
        );
        self.db.put_job_logged(&job, &line, true)?;
        // Lists name a job's current stage.
        self.events.job(&job, None);
        Ok(())
    }

    /// Log whole lines of a build's or a compose setup's output and follow its scripts'
    /// progress. A line that begins a build stage starts it before the line, and ends the
    /// scripts' progress; the trace's lines stay out of the log.
    pub(crate) fn log_script_output(
        &self,
        id: &str,
        text: &str,
        progress: &mut ScriptProgress,
    ) -> Result<()> {
        let at = now();
        let mut kept = String::new();
        for line in text.split_inclusive('\n') {
            if let Some(stage) = progress.builds().then(|| buildkit_stage(line)).flatten() {
                self.log_text(id, &std::mem::take(&mut kept))?;
                progress.end();
                self.save_script(id, progress)?;
                self.begin_stage(id, stage)?;
            }
            if progress.read(line, at) {
                kept.push_str(line);
            }
        }
        self.log_text(id, &kept)?;
        self.save_script(id, progress)
    }

    fn log_text(&self, id: &str, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        // The log adds the line end itself.
        self.append_log(id, text.strip_suffix('\n').unwrap_or(text))
    }
}

#[cfg(test)]
#[path = "stages_tests.rs"]
mod tests;
