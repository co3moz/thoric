//! Requests for a person. An agent that reaches what only a human may do in its desktop, such as
//! a CAPTCHA, a second sign-in factor, a license window or a sign-in screen, asks with the MCP
//! tool `request_human` and waits. The dashboard shows the request on every page and above the
//! container's desktop until someone answers Done or Decline, with an optional reply, and the
//! waiting call returns that answer. A request belongs to the container and the MCP channel it
//! was asked through, and deleting the container ends it. Requests live in memory: a manager
//! restart ends them.
use crate::{
    App,
    error::{Error, Result},
    events::Hub,
    model::{Container, now},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rmcp::{
    Peer, RoleServer,
    model::{CallToolResult, ProgressNotificationParam, ProgressToken},
    service::RequestContext,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};
use tokio_util::sync::CancellationToken;

/// The longest message an agent may leave, in characters.
pub(crate) const MESSAGE_LIMIT: usize = 2000;
/// The longest reply a person may give, in characters.
pub(crate) const REPLY_LIMIT: usize = 1000;
/// Requests that may wait for an answer at once.
const PENDING_LIMIT: usize = 64;
/// How long an answered request can still be read by its id, so a waiting agent that timed out
/// learns the answer.
const ANSWERED_KEPT: u64 = 3600;
const ANSWERED_LIMIT: usize = 256;
/// How long one call waits when the agent does not say, below the minute MCP clients often allow.
pub(crate) const DEFAULT_WAIT: u64 = 45;
/// The longest one call waits; the agent calls again with the request's id to wait longer.
pub(crate) const MAX_WAIT: u64 = 300;
/// How often a waiting call reports progress to a client that asked for it.
const PROGRESS_EVERY: Duration = Duration::from_secs(10);
/// The reply a request of a deleted container gets, and one of a deleted endpoint.
const CONTAINER_DELETED: &str = "The container was deleted, which ended this request for a person.";
const ENDPOINT_DELETED: &str =
    "The MCP endpoint was deleted, which ended this request for a person.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    /// Nobody answered yet.
    Pending,
    /// Someone did what was asked.
    Done,
    /// Someone would not, or could not.
    Declined,
}

#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub(crate) struct HumanRequest {
    pub id: String,
    pub container_id: String,
    /// The container's name when the agent asked, kept should it be deleted meanwhile.
    pub container_name: String,
    pub message: String,
    /// The MCP endpoint the agent asked through; `None` for the administrator's `/mcp`.
    pub endpoint: Option<String>,
    pub status: Status,
    pub reply: Option<String>,
    pub created_at: u64,
    pub answered_at: Option<u64>,
}

/// The open requests and those answered within the last hour, oldest first.
#[derive(Default)]
pub(crate) struct Requests {
    items: Mutex<Vec<HumanRequest>>,
    /// Woken whenever a request is answered.
    answered: Notify,
}

impl Requests {
    /// Ask on behalf of an agent, or join the request still waiting with the same message for the
    /// same container through the same endpoint, as an agent does that lost its call and asks
    /// again. Another endpoint's agent asks a request of its own, whose answer it alone reads.
    pub(crate) fn ask(
        &self,
        container: &Container,
        message: &str,
        endpoint: Option<&str>,
        hub: &Hub,
    ) -> Result<HumanRequest> {
        let message = message.trim();
        if message.is_empty() || message.chars().count() > MESSAGE_LIMIT {
            return Err(bad(format!(
                "message must say what is needed in 1 to {MESSAGE_LIMIT} characters"
            )));
        }
        let mut items = self.items.lock().unwrap();
        prune(&mut items);
        if let Some(open) = items.iter().find(|request| {
            request.status == Status::Pending
                && request.container_id == container.id
                && request.endpoint.as_deref() == endpoint
                && request.message == message
        }) {
            return Ok(open.clone());
        }
        if pending(&items).count() >= PENDING_LIMIT {
            return Err(Error(
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "{PENDING_LIMIT} requests already wait for a person; answer or decline some in the dashboard first"
                ),
            ));
        }
        let request = HumanRequest {
            id: uuid::Uuid::new_v4().simple().to_string(),
            container_id: container.id.clone(),
            container_name: container.spec.name.clone(),
            message: message.to_owned(),
            endpoint: endpoint.map(str::to_owned),
            status: Status::Pending,
            reply: None,
            created_at: now(),
            answered_at: None,
        };
        items.push(request.clone());
        hub.publish(json!({"type": "human-request", "request": request}));
        Ok(request)
    }

    /// A person's answer to a request that still waits.
    pub(crate) fn answer(
        &self,
        id: &str,
        answer: Answer,
        reply: Option<&str>,
        hub: &Hub,
    ) -> Result<HumanRequest> {
        let reply = reply.map(str::trim).filter(|reply| !reply.is_empty());
        if reply.is_some_and(|reply| reply.chars().count() > REPLY_LIMIT) {
            return Err(bad(format!("A reply has at most {REPLY_LIMIT} characters")));
        }
        let answered = {
            let mut items = self.items.lock().unwrap();
            prune(&mut items);
            let request = items
                .iter_mut()
                .find(|request| request.id == id)
                .ok_or_else(|| {
                    Error(
                        StatusCode::NOT_FOUND,
                        "This request is no longer open".into(),
                    )
                })?;
            if request.status != Status::Pending {
                return Err(Error(
                    StatusCode::CONFLICT,
                    "This request was answered already".into(),
                ));
            }
            request.status = match answer {
                Answer::Done => Status::Done,
                Answer::Decline => Status::Declined,
            };
            request.reply = reply.map(str::to_owned);
            request.answered_at = Some(now());
            request.clone()
        };
        self.answered.notify_waiters();
        hub.publish(json!({"type": "human-request-answered", "request": answered}));
        Ok(answered)
    }

    /// Decline the requests that wait in a deleted container, so they leave the dashboard and the
    /// limit, and a call that waits on one returns saying why.
    pub(crate) fn end_container(&self, container_id: &str, hub: &Hub) {
        self.end(
            |request| request.container_id == container_id,
            CONTAINER_DELETED,
            hub,
        );
    }

    /// Decline the requests a deleted MCP endpoint asked, which nobody waits on any more and an
    /// endpoint made later with its name must not join.
    pub(crate) fn end_endpoint(&self, endpoint: &str, hub: &Hub) {
        self.end(
            |request| request.endpoint.as_deref() == Some(endpoint),
            ENDPOINT_DELETED,
            hub,
        );
    }

    fn end(&self, which: impl Fn(&HumanRequest) -> bool, reply: &str, hub: &Hub) {
        let ended = {
            let mut items = self.items.lock().unwrap();
            let at = now();
            items
                .iter_mut()
                .filter(|request| request.status == Status::Pending && which(request))
                .map(|request| {
                    request.status = Status::Declined;
                    request.reply = Some(reply.into());
                    request.answered_at = Some(at);
                    request.clone()
                })
                .collect::<Vec<_>>()
        };
        if ended.is_empty() {
            return;
        }
        self.answered.notify_waiters();
        for request in ended {
            hub.publish(json!({"type": "human-request-answered", "request": request}));
        }
    }

    /// The requests that wait for a person, oldest first.
    pub(crate) fn pending(&self) -> Vec<HumanRequest> {
        let mut items = self.items.lock().unwrap();
        prune(&mut items);
        pending(&items).cloned().collect()
    }

    fn get(&self, id: &str) -> Option<HumanRequest> {
        self.items
            .lock()
            .unwrap()
            .iter()
            .find(|request| request.id == id)
            .cloned()
    }

    /// Wait up to `wait` for the request to be answered, and return it as it is then. The wait
    /// also ends when the client gives the call up; the request stays open for a person.
    pub(crate) async fn wait(
        &self,
        id: &str,
        wait: Duration,
        caller: &Caller,
        hub: &Hub,
    ) -> Result<HumanRequest> {
        let started = Instant::now();
        let deadline = started + wait;
        let mut progress = tokio::time::interval(PROGRESS_EVERY);
        progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let notified = self.answered.notified();
            tokio::pin!(notified);
            // Registered before the look, so an answer between the two is not missed.
            notified.as_mut().enable();
            let request = self.get(id).ok_or_else(|| unknown(id))?;
            if request.status != Status::Pending
                || Instant::now() >= deadline
                || caller.cancelled.is_cancelled()
            {
                return Ok(request);
            }
            tokio::select! {
                () = &mut notified => {}
                () = tokio::time::sleep_until(deadline) => {}
                () = hub.closed() => return Err(Error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The manager is shutting down, which ends every open request for a person. Ask again with message once it answers, if a person is still needed.".into(),
                )),
                () = caller.cancelled.cancelled() => {}
                _ = progress.tick() => caller.report(started.elapsed(), wait).await,
            }
        }
    }
}

fn pending(items: &[HumanRequest]) -> impl Iterator<Item = &HumanRequest> {
    items
        .iter()
        .filter(|request| request.status == Status::Pending)
}

/// Forget answers older than an hour, and the oldest beyond the limit.
fn prune(items: &mut Vec<HumanRequest>) {
    let now = now();
    items.retain(|request| {
        request
            .answered_at
            .is_none_or(|at| now.saturating_sub(at) < ANSWERED_KEPT)
    });
    let answered = items.len() - pending(items).count();
    let mut extra = answered.saturating_sub(ANSWERED_LIMIT);
    items.retain(|request| {
        if extra > 0 && request.status != Status::Pending {
            extra -= 1;
            return false;
        }
        true
    });
}

fn bad(message: String) -> Error {
    Error(StatusCode::BAD_REQUEST, message)
}

fn unknown(id: &str) -> Error {
    Error(
        StatusCode::NOT_FOUND,
        format!(
            "No request {id} for a person is open or was answered within the hour for this container through this MCP channel. A manager restart ends every open request; ask again with message if a person is still needed."
        ),
    )
}

/// How a person answered.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Answer {
    Done,
    Decline,
}

/// Who waits on a request: the MCP call, which hears progress when its client asked for it and
/// may be given up, and the endpoint it came through.
#[derive(Clone, Default)]
pub(crate) struct Caller {
    endpoint: Option<String>,
    progress: Option<(ProgressToken, Peer<RoleServer>)>,
    cancelled: CancellationToken,
}

impl Caller {
    pub(crate) fn new(context: &RequestContext<RoleServer>, endpoint: Option<&str>) -> Self {
        Self {
            endpoint: endpoint.map(str::to_owned),
            progress: context
                .meta
                .get_progress_token()
                .map(|token| (token, context.peer.clone())),
            cancelled: context.ct.clone(),
        }
    }
    /// The MCP endpoint the call came through; `None` for the administrator's `/mcp`.
    pub(crate) fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }
    /// Tell a client that asked for progress how long the call has waited, so one that resets
    /// its timeout on progress keeps the call open.
    async fn report(&self, waited: Duration, wait: Duration) {
        self.report_waiting(
            waited,
            wait,
            "Waiting for a person to answer in the Thoric dashboard",
        )
        .await;
    }
    /// [`Caller::report`] for another wait, such as `get_job`'s for a job to end.
    pub(crate) async fn report_waiting(&self, waited: Duration, wait: Duration, message: &str) {
        if let Some((token, peer)) = &self.progress {
            let progress = ProgressNotificationParam::new(token.clone(), waited.as_secs_f64())
                .with_total(wait.as_secs_f64())
                .with_message(message);
            let _ = peer.notify_progress(progress).await;
        }
    }
    /// Resolves once the client gives the call up.
    pub(crate) async fn given_up(&self) {
        self.cancelled.cancelled().await;
    }
}

/// The arguments of `request_human`, beside its container.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ask {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

/// The MCP tool `request_human` for `container_id`: ask, or keep waiting on an earlier request.
pub(crate) async fn request(
    app: &App,
    container_id: &str,
    args: Value,
    caller: Caller,
) -> Result<CallToolResult> {
    let ask: Ask =
        serde_json::from_value(args).map_err(|e| bad(format!("Invalid tool arguments: {e}")))?;
    let wait = Duration::from_secs(ask.wait_seconds.unwrap_or(DEFAULT_WAIT).min(MAX_WAIT));
    let id = match (ask.message, ask.request_id) {
        (Some(message), None) => {
            let container = app.container(container_id)?;
            let request = app.human.ask(
                &container,
                &message,
                caller.endpoint.as_deref(),
                &app.events,
            )?;
            // Asked again, the request still waiting is joined, and its key sends nothing more.
            crate::notify::send(app, crate::notify::notices::human_request(&request));
            request.id
        }
        (None, Some(id)) => {
            // Only a request of this container asked through the same endpoint, so an agent
            // cannot read another endpoint's.
            match app.human.get(&id) {
                Some(request)
                    if request.container_id == container_id
                        && request.endpoint.as_deref() == caller.endpoint() =>
                {
                    id
                }
                _ => return Err(unknown(&id)),
            }
        }
        _ => {
            return Err(bad(
                "Pass message to ask a person, or request_id to keep waiting on a request, not both".into(),
            ));
        }
    };
    let request = app.human.wait(&id, wait, &caller, &app.events).await?;
    let mut result = json!({
        "request_id": request.id,
        "status": request.status,
        "message": request.message,
        "reply": request.reply,
        "asked_at": request.created_at,
        "answered_at": request.answered_at,
    });
    if request.status == Status::Pending {
        result["next"] = json!(
            "Nobody answered yet. Call request_human again with this request_id, without message, to keep waiting."
        );
    }
    Ok(CallToolResult::structured(result))
}

/// `GET /api/human-requests`: the requests that wait for a person, oldest first.
pub async fn list(State(app): State<Arc<App>>) -> Json<Value> {
    Json(json!({"items": app.human.pending()}))
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerBody {
    answer: Answer,
    #[serde(default)]
    reply: Option<String>,
}

/// `POST /api/human-requests/{id}/answer`: answer Done or Decline, with an optional reply.
pub async fn answer(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<HumanRequest>> {
    Ok(Json(app.human.answer(
        &id,
        body.answer,
        body.reply.as_deref(),
        &app.events,
    )?))
}

#[cfg(test)]
#[path = "human_tests.rs"]
mod tests;
