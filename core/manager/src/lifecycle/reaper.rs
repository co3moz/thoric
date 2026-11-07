//! The lifecycle's one actor. Every [`EVERY`] it reads the records with a policy and Docker's
//! running desktops, and carries out what is due one container at a time: [`act`] takes the
//! container's lock as a request does, without waiting for it, reads the record and the desktop
//! again under it and starts a job that keeps the lock, so nothing changes the container between
//! the check and the stop or the delete. A busy lock waits for the next round. The jobs are the
//! routes' own stop and delete, under kinds that say why: [`IDLE_STOP`], [`EXPIRY_STOP`] and
//! [`EXPIRY_DELETE`]. A job that outlasts [`JOB_WAIT`] is followed by the rounds after, and one
//! that failed leaves its container alone for [`RETRY_AFTER`]. While a restore runs, which saves
//! the records of the containers it brings back before it creates them, the reaper does nothing.
use super::{Due, OnExpiry, busy, due_for};
use crate::{
    App,
    jobs::Outcome,
    model::{CONTAINERS, Container, LABEL},
    reported::Reported,
};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::oneshot::error::TryRecvError, time::Instant};

/// How often the reaper looks.
pub(super) const EVERY: Duration = Duration::from_secs(30);
/// How long the reaper waits for one of its jobs before it goes on with the next container, a
/// little longer than a Docker command may take; the rounds after collect the job's outcome.
const JOB_WAIT: Duration = Duration::from_secs(150);
/// How long a container whose stop or delete failed is left alone, so a failure that lasts does
/// not fill the job list with a job every round.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(15 * 60);

/// A running desktop stopped because nothing used it.
pub(crate) const IDLE_STOP: &str = "idle-stop";
/// A container stopped at its expiry.
pub(crate) const EXPIRY_STOP: &str = "expiry-stop";
/// A container deleted at its expiry.
pub(crate) const EXPIRY_DELETE: &str = "expiry-delete";
/// An instance of a scale set deleted because nothing used it.
pub(crate) const IDLE_DELETE: &str = "idle-delete";

/// What a failed round reports itself as.
const ROUND: &str = "Container lifecycle";
fn subject(id: &str) -> String {
    format!("Container {id} lifecycle")
}

/// Look every [`EVERY`] for as long as the manager runs. The first round comes at once, which
/// carries out the expiries that passed while the manager was down; an idle stop counts from the
/// start.
pub(crate) fn spawn(app: Arc<App>) {
    tokio::spawn(async move {
        let mut reaper = Reaper::default();
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let result = reaper.round(&app).await;
            reaper.reported.outcome(ROUND, &result);
        }
    });
}

/// What the reaper remembers between rounds.
#[derive(Default)]
pub(super) struct Reaper {
    reported: Reported,
    /// When each container's last stop or delete failed.
    failed: HashMap<String, Instant>,
    /// The outcomes of the jobs still running after [`JOB_WAIT`], by container.
    pending: HashMap<String, Outcome>,
}

/// What [`act`] did.
pub(super) enum Acted {
    /// Nothing: the lock was busy, or nothing was due any more.
    Nothing,
    /// It started a job, which completed (its ID) or failed (an error that names it).
    Ended(Result<String>),
    /// It started this job, which still runs after [`JOB_WAIT`].
    Running(String, Outcome),
}

impl Reaper {
    /// One look at every container with a policy.
    pub(super) async fn round(&mut self, app: &Arc<App>) -> Result<()> {
        self.collect();
        let records: Vec<Container> = app.db.list(CONTAINERS)?;
        let known = |id: &str| records.iter().any(|record| record.id == id);
        app.lifecycle.retain(known);
        self.failed
            .retain(|id, at| at.elapsed() < RETRY_AFTER && known(id));
        self.reported
            .retain(|line| line == ROUND || records.iter().any(|r| line == subject(&r.id)));
        let sets = crate::scale::all(app)?;
        if (records
            .iter()
            .all(|record| record.spec.lifecycle.is_never())
            && sets.iter().all(|set| set.idle_delete_minutes == 0))
            || restoring(app)?
        {
            return Ok(());
        }
        let running = running(app).await?;
        for id in self.due(app, &records, &sets, &running, crate::model::now()) {
            match act(app, &id).await {
                Ok(Acted::Nothing) => {}
                Ok(Acted::Ended(outcome)) => self.settle(&id, outcome.map(drop)),
                Ok(Acted::Running(job, outcome)) => {
                    tracing::debug!(
                        "Job {job} on container {id} still runs; a later round settles it"
                    );
                    self.pending.insert(id, outcome);
                }
                Err(error) => self.settle(&id, Err(error)),
            }
        }
        Ok(())
    }

    /// Note how a stop or delete of the container `id` ended: a failure is logged as a warning
    /// once, and the container is left alone for [`RETRY_AFTER`].
    fn settle(&mut self, id: &str, outcome: Result<()>) {
        if outcome.is_err() {
            self.failed.insert(id.to_owned(), Instant::now());
        }
        self.reported.outcome(&subject(id), &outcome);
    }

    /// Settle the jobs that were still running and have ended since.
    pub(super) fn collect(&mut self) {
        for (id, mut outcome) in std::mem::take(&mut self.pending) {
            match outcome.try_recv() {
                Err(TryRecvError::Empty) => {
                    self.pending.insert(id, outcome);
                }
                Ok(Ok(_)) => self.settle(&id, Ok(())),
                Ok(Err(error)) => self.settle(&id, Err(anyhow!(error))),
                Err(TryRecvError::Closed) => {
                    self.settle(&id, Err(anyhow!("the job ended without an outcome")))
                }
            }
        }
    }

    /// The containers whose policy asks for something at `now`, by the round's reading of the
    /// records and of the desktops that run. A desktop whose CPU is busy is noted as used here,
    /// so its idle time runs from the last round that found it busy.
    pub(super) fn due(
        &self,
        app: &App,
        records: &[Container],
        sets: &[crate::scale::ScaleSet],
        running: &HashSet<String>,
        now: u64,
    ) -> Vec<String> {
        let mut due: Vec<(Option<u32>, String)> = records
            .iter()
            .filter(|record| {
                !record.spec.lifecycle.is_never()
                    || crate::scale::instance_of(sets, &record.id)
                        .is_some_and(|(set, _)| set.idle_delete_minutes > 0)
            })
            .filter_map(|record| {
                let up = running.contains(&record.container);
                if up && busy(app, &record.id) {
                    app.lifecycle.touch(&record.id);
                }
                if self.failed.contains_key(&record.id) || self.pending.contains_key(&record.id) {
                    return None;
                }
                due_for(app, record, sets, up, now).map(|_| {
                    let number = crate::scale::instance_of(sets, &record.id).map(|(_, n)| n);
                    (number, record.id.clone())
                })
            })
            .collect();
        // A set's highest numbers go first, so the instances that stay keep the lowest.
        due.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        due.into_iter().map(|(_, id)| id).collect()
    }
}

/// Whether a restore runs, which saves the records of the containers it brings back before it
/// creates them one at a time: one it has yet to create is not to be stopped or deleted. Read
/// after the records, so the round never acts on a record a running restore saved.
fn restoring(app: &App) -> Result<bool> {
    Ok(app
        .db
        .active_jobs()?
        .iter()
        .any(|job| job.kind == "restore"))
}

/// The names of the Docker containers of this manager's desktops that run.
async fn running(app: &App) -> Result<HashSet<String>> {
    let names = app
        .docker(&[
            "ps",
            "--filter",
            &format!("label={LABEL}=true"),
            "--format",
            "{{.Names}}",
        ])
        .await?;
    Ok(String::from_utf8_lossy(&names)
        .lines()
        .map(str::to_owned)
        .collect())
}

/// Carry out what the container's policy asks for, if it still does once its lock is held: the
/// one place the lifecycle stops or deletes a container. Nothing happens when the lock is busy,
/// which the next round tries again, or when nothing is due any more, such as after a settings
/// change gave it a later expiry. A job still running after [`JOB_WAIT`] goes on, holding the
/// container, and comes back with its outcome to collect.
pub(super) async fn act(app: &Arc<App>, id: &str) -> Result<Acted> {
    let resource = format!("container:{id}");
    let Ok(held) = app.resource_lock(&resource).try_lock_owned() else {
        return Ok(Acted::Nothing);
    };
    // Read again under the lock, which every change of the record holds.
    let Ok(record) = app.container(id) else {
        return Ok(Acted::Nothing);
    };
    let info = app.optional_inspect("container", &record.container).await?;
    let running = info
        .as_ref()
        .is_some_and(|info| info["State"]["Running"] == true);
    let now = crate::model::now();
    let sets = crate::scale::all(app)?;
    let Some(due) = due_for(app, &record, &sets, running, now) else {
        return Ok(Acted::Nothing);
    };
    let kind = match due {
        Due::IdleStop(_) => IDLE_STOP,
        Due::IdleDelete(_) => IDLE_DELETE,
        Due::Expiry(OnExpiry::Stop) => EXPIRY_STOP,
        Due::Expiry(OnExpiry::Delete) => EXPIRY_DELETE,
    };
    let owner = id.to_owned();
    let (job, mut outcome) = crate::jobs::start_holding(
        app.clone(),
        kind,
        resource,
        held,
        move |app, job_id| async move { carry_out(&app, &owner, due, &job_id).await },
    )
    .map_err(|error| anyhow!(error.1))?;
    tracing::info!(
        "{} \"{}\" ({id}), job {job}",
        match due {
            Due::IdleStop(_) => "Stopping the idle container",
            Due::IdleDelete(_) => "Deleting the idle instance",
            Due::Expiry(OnExpiry::Stop) => "Stopping the expired container",
            Due::Expiry(OnExpiry::Delete) => "Deleting the expired container",
        },
        record.spec.name,
    );
    Ok(match tokio::time::timeout(JOB_WAIT, &mut outcome).await {
        Ok(Ok(Ok(_))) => Acted::Ended(Ok(job)),
        Ok(Ok(Err(error))) => Acted::Ended(Err(anyhow!("job {job} failed: {error}"))),
        Ok(Err(_)) => Acted::Ended(Err(anyhow!("job {job} ended without an outcome"))),
        Err(_) => Acted::Running(job, outcome),
    })
}

/// The work of the reaper's job, which holds the container from [`act`]'s check on. Once it is
/// done, it sends the lifecycle's one notice (`container_lifecycle`); a job that fails sends
/// `job_failed` instead, as every failed job does.
async fn carry_out(app: &App, id: &str, due: Due, job_id: &str) -> Result<Value> {
    let record = app.container(id).map_err(|error| anyhow!(error.1))?;
    let result = carried_out(app, &record, due, job_id).await?;
    crate::notify::send(
        app,
        crate::notify::notices::container_lifecycle(&record, due, job_id),
    );
    Ok(result)
}

/// The stop or delete itself.
async fn carried_out(app: &App, record: &Container, due: Due, job_id: &str) -> Result<Value> {
    let (id, name) = (record.id.as_str(), &record.spec.name);
    match due {
        Due::IdleStop(minutes) => {
            app.append_log(
                job_id,
                &format!("Stopping \"{name}\": nothing used its desktop for {minutes} minutes."),
            )?;
            crate::api::run_action(app, id, "stop", job_id).await?;
            app.append_log(
                job_id,
                "Stopped. Its files stay; starting it again gives it the same idle time.",
            )?;
            Ok(json!({"container_id": id, "idle_minutes": minutes}))
        }
        Due::Expiry(OnExpiry::Stop) => {
            let info = app.optional_inspect("container", &record.container).await?;
            if info.is_some_and(|info| info["State"]["Running"] == true) {
                app.append_log(job_id, &format!("\"{name}\" expired; stopping it."))?;
                crate::api::run_action(app, id, "stop", job_id).await?;
            } else {
                app.append_log(
                    job_id,
                    &format!("\"{name}\" expired. It was stopped already."),
                )?;
            }
            // Carried out, so a start keeps the desktop running.
            let mut record = app.container(id).map_err(|error| anyhow!(error.1))?;
            record.spec.lifecycle.expires_at = None;
            app.db.put(CONTAINERS, id, &record)?;
            app.append_log(
                job_id,
                "Cleared its expiry, so it keeps running once it is started again.",
            )?;
            Ok(json!({"container_id": id, "expired": true}))
        }
        Due::IdleDelete(minutes) => {
            app.append_log(
                job_id,
                &format!(
                    "Deleting \"{name}\": nothing used this instance of its scale set for {minutes} minutes."
                ),
            )?;
            let removed = app.remove_container(record, job_id, true).await?;
            Ok(crate::api::removal(id, name, true, removed))
        }
        Due::Expiry(OnExpiry::Delete) => {
            app.append_log(job_id, &format!("\"{name}\" expired; deleting it."))?;
            let volumes = record.spec.lifecycle.delete_volumes;
            let removed = app.remove_container(record, job_id, volumes).await?;
            Ok(crate::api::removal(id, name, volumes, removed))
        }
    }
}

#[cfg(test)]
#[path = "reaper_tests.rs"]
mod tests;
