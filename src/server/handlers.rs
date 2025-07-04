pub mod status_get;
pub mod trackers_bulk_remove;
pub mod trackers_clear_revisions;
pub mod trackers_create;
pub mod trackers_create_revision;
pub mod trackers_get;
pub mod trackers_get_revision;
pub mod trackers_list;
pub mod trackers_list_revisions;
pub mod trackers_remove;
pub mod trackers_remove_revision;
pub mod trackers_update;

use crate::server::Status;
use retrack_types::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    trackers::{
        ApiTarget, EmailAction, ExtractorEngine, PageTarget, ServerLogAction, TargetRequest,
        Tracker, TrackerAction, TrackerConfig, TrackerCreateParams, TrackerDataRevision,
        TrackerDataValue, TrackerTarget, TrackerUpdateParams, WebhookAction,
    },
};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Retrack",
        license(
            name = "AGPL-3.0",
            url = "https://github.com/secutils-dev/retrack/blob/main/LICENSE"
        )
    ),
    paths(
        status_get::status_get,
        trackers_list::trackers_list,
        trackers_get::trackers_get,
        trackers_create::trackers_create,
        trackers_update::trackers_update,
        trackers_remove::trackers_remove,
        trackers_bulk_remove::trackers_bulk_remove,
        trackers_list_revisions::trackers_list_revisions,
        trackers_create_revision::trackers_create_revision,
        trackers_clear_revisions::trackers_clear_revisions,
        trackers_get_revision::trackers_get_revision,
        trackers_remove_revision::trackers_remove_revision
    ),
    components(schemas(
        ApiTarget,
        EmailAction,
        ExtractorEngine,
        SchedulerJobConfig,
        SchedulerJobRetryStrategy,
        Status,
        TargetRequest,
        Tracker,
        TrackerAction,
        TrackerConfig,
        TrackerCreateParams,
        TrackerDataRevision,
        TrackerDataValue,
        TrackerTarget,
        TrackerUpdateParams,
        PageTarget,
        ServerLogAction,
        WebhookAction
    ))
)]
pub(super) struct RetrackOpenApi;
