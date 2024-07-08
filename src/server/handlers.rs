pub mod status_get;
pub mod trackers_clear_revisions;
pub mod trackers_create;
pub mod trackers_get;
pub mod trackers_list;
pub mod trackers_list_revisions;
pub mod trackers_remove;
pub mod trackers_update;

use crate::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    server::Status,
    trackers::{Tracker, TrackerConfig, TrackerCreateParams, TrackerUpdateParams},
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
        trackers_list_revisions::trackers_list_revisions,
        trackers_clear_revisions::trackers_clear_revisions
    ),
    components(schemas(
        Status,
        Tracker,
        TrackerConfig,
        TrackerCreateParams,
        TrackerUpdateParams,
        SchedulerJobConfig,
        SchedulerJobRetryStrategy
    ))
)]
pub(super) struct RetrackOpenApi;
