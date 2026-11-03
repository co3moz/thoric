//! The notices Thoric sends, one constructor per kind. They are English, as everything the
//! manager writes for a person outside the dashboard is, and never carry a secret value or what a
//! tool returned.
use super::{Kind, Notice};
use crate::{App, human::HumanRequest, model::Job};

/// An agent asked for a person: its message, in the container it asked for, linking to that
/// desktop, where the request waits under the toolbar. One notice per request, however often the
/// agent waits on it again.
pub(crate) fn human_request(request: &HumanRequest) -> Notice {
    Notice {
        kind: Kind::HumanRequest,
        title: format!("{} needs a person", request.container_name),
        body: request.message.clone(),
        container: Some(request.container_name.clone()),
        link: Some(format!("#/desktop/{}", request.container_id)),
        key: Some(format!("human_request:{}", request.id)),
    }
}

/// A job failed: its kind and what it worked on. A cancelled job sends nothing, and its error,
/// which a script's output can fill, stays in its log.
pub(crate) fn job_failed(app: &App, job: &Job) -> Notice {
    let container = job
        .resource
        .as_deref()
        .and_then(|resource| resource.strip_prefix("container:"))
        .and_then(|id| app.container(id).ok())
        .map(|container| container.spec.name);
    let subject = job.name.clone().or_else(|| container.clone()).or_else(|| {
        let (kind, name) = job.resource.as_deref()?.split_once(':')?;
        match kind {
            "image" => app
                .db
                .get::<crate::model::ImageRecord>("image", name)
                .ok()
                .map(|image| image.name),
            "volume" => Some(name.to_owned()),
            _ => None,
        }
    });
    let body = match &subject {
        Some(subject) => format!("The {} job of {subject} failed.", job.kind),
        None => format!("A {} job failed.", job.kind),
    };
    Notice {
        kind: Kind::JobFailed,
        title: format!("Job failed: {}", job.kind),
        body: format!("{body} Its log is on the Jobs page."),
        container,
        link: Some("#/jobs".into()),
        key: Some(format!("job_failed:{}", job.id)),
    }
}

/// A newer release than this manager is out, linking to its notes.
pub(crate) fn update_available(version: &str, release_url: &str) -> Notice {
    Notice {
        kind: Kind::UpdateAvailable,
        title: format!("Thoric {version} is available"),
        body: format!("This manager runs Thoric {}.", crate::releases::VERSION),
        container: None,
        link: Some(release_url.to_owned()),
        key: Some(format!("update_available:{version}")),
    }
}

/// The Management page's test.
pub(crate) fn test() -> Notice {
    Notice {
        kind: Kind::Test,
        title: "Thoric test notification".into(),
        body: "Notifications from this Thoric installation reach you here.".into(),
        container: None,
        link: Some("#/settings".into()),
        key: None,
    }
}
