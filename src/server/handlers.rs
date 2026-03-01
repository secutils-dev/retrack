pub mod status_get;
pub mod trackers_bulk_remove;
pub mod trackers_clear_revisions;
pub mod trackers_create;
pub mod trackers_create_revision;
pub mod trackers_debug;
pub mod trackers_debug_existing;
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
        ActionDebugInfo, ActionDestinationDebugInfo, ApiRequestDebugInfo, ApiTarget,
        ApiTrackerDebugResult, AutoParseDebugInfo, EmailAction, ExtractorEngine, PageLogEntry,
        PageTarget, PageTrackerDebugResult, RenderedEmailDebugInfo, ScriptDebugInfo,
        ServerLogAction, TargetRequest, Tracker, TrackerAction, TrackerConfig, TrackerCreateParams,
        TrackerDataRevision, TrackerDataValue, TrackerDebugExistingParams, TrackerDebugParams,
        TrackerDebugResult, TrackerDebugTargetResult, TrackerTarget, TrackerUpdateParams,
        WebhookAction, WebhookDestinationDebugInfo,
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
        trackers_remove_revision::trackers_remove_revision,
        trackers_debug::trackers_debug,
        trackers_debug_existing::trackers_debug_existing
    ),
    components(schemas(
        ActionDebugInfo,
        ActionDestinationDebugInfo,
        ApiRequestDebugInfo,
        ApiTarget,
        ApiTrackerDebugResult,
        AutoParseDebugInfo,
        EmailAction,
        ExtractorEngine,
        PageLogEntry,
        PageTarget,
        PageTrackerDebugResult,
        RenderedEmailDebugInfo,
        SchedulerJobConfig,
        SchedulerJobRetryStrategy,
        ScriptDebugInfo,
        ServerLogAction,
        Status,
        TargetRequest,
        Tracker,
        TrackerAction,
        TrackerConfig,
        TrackerCreateParams,
        TrackerDataRevision,
        TrackerDataValue,
        TrackerDebugExistingParams,
        TrackerDebugParams,
        TrackerDebugResult,
        TrackerDebugTargetResult,
        TrackerTarget,
        TrackerUpdateParams,
        WebhookAction,
        WebhookDestinationDebugInfo
    ))
)]
pub(super) struct RetrackOpenApi;
