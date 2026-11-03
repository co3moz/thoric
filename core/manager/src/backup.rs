//! Backing up the whole installation into one archive, and restoring it into another.
//!
//! A backup is a `.tar.gz` the dashboard downloads as it is made (see `format`): a manifest that
//! names what it holds, the records of the included kinds (see `manifest`), and the parts the
//! person chose, each streamed from Docker or a volume helper without a copy on the manager's
//! disk: the saved images as `docker save` streams, each container's files as a `docker save` of
//! a commit of its container layer, and the named volumes as tar streams. Catalog images stay
//! out; the installation they return to downloads them when it needs them. Settings stay out too:
//! they live in `.env`, outside the manager.
//!
//! A restore reads the upload as it arrives. The manifest and records come first and are checked
//! before anything is created (see `review`): a backup whose names this installation already uses
//! is refused whole. Images are loaded through the same checks an image import uses and tagged
//! with Thoric's own names, volumes are unpacked into new volumes as the desktop user, and
//! anything this restore created is removed again when the upload fails. Once the upload has
//! ended the records are saved together, and each container whose files came along is created
//! from them with its machine identity, and left stopped.
//!
//! Only the dashboard and the HTTP API reach this; MCP has no tool for it, since a restore creates
//! containers and images at once and a backup carries the installation's secrets.
mod export;
mod format;
mod manifest;
mod remap;
mod restore;
mod review;

pub(crate) use export::export;
pub(crate) use manifest::Included;
pub(crate) use remap::switch_off_disabled;
pub(crate) use restore::Upload;
pub(crate) use restore::{check, restore};

use crate::{
    App,
    error::{Error, Result},
};
use axum::http::StatusCode;

/// The resource a backup and a restore hold, so only one of them runs at a time.
const RESOURCE: &str = "installation";

/// Take the installation for a backup or a restore, or refuse at once while another holds it.
fn installation(app: &App) -> Result<crate::jobs::Exclusive> {
    app.resource_lock(RESOURCE).try_lock_owned().map_err(|_| {
        Error(
            StatusCode::CONFLICT,
            "A backup or a restore is already running. Try again when it finishes.".into(),
        )
    })
}

/// A file name for a backup made at `seconds` since 1970, in UTC.
fn file_name(seconds: u64) -> String {
    // Howard Hinnant's days-to-civil conversion.
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let time = seconds % 86_400;
    format!(
        "thoric-backup-{year:04}{month:02}{day:02}-{:02}{:02}{:02}.tar.gz",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
