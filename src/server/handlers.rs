pub mod status_get;
pub mod trackers_bulk_get;
pub mod trackers_bulk_remove;
pub mod trackers_clear_all_execution_logs;
pub mod trackers_clear_execution_logs;
pub mod trackers_clear_revisions;
pub mod trackers_create;
pub mod trackers_create_revision;
pub mod trackers_debug;
pub mod trackers_debug_existing;
pub mod trackers_get;
pub mod trackers_get_revision;
pub mod trackers_import_revisions;
pub mod trackers_list;
pub mod trackers_list_execution_logs;
pub mod trackers_list_execution_logs_batch;
pub mod trackers_list_revisions;
pub mod trackers_list_revisions_batch;
pub mod trackers_remove;
pub mod trackers_remove_revision;
pub mod trackers_update;

use crate::server::{DatabaseStatus, Status};
use retrack_types::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    trackers::{
        ActionDebugInfo, ActionDestinationDebugInfo, ApiRequestDebugInfo, ApiTarget,
        ApiTrackerDebugResult, AutoParseDebugInfo, EmailAction, ExtractorEngine, Page,
        PageLogEntry, PageTarget, PageTrackerDebugResult, RenderedEmailDebugInfo, ScriptDebugInfo,
        ServerLogAction, SortOrder, TargetRequest, Tracker, TrackerAction, TrackerConfig,
        TrackerCreateParams, TrackerDataRevision, TrackerDataRevisionImportParams,
        TrackerDataRevisionImportResult, TrackerDataValue, TrackerDebugExistingParams,
        TrackerDebugParams, TrackerDebugResult, TrackerDebugTargetResult, TrackerExecutionLog,
        TrackerExecutionLogPhase, TrackerExecutionLogStatus, TrackerListExecutionLogsBatchParams,
        TrackerListRevisionsBatchParams, TrackerTarget, TrackerUpdateParams, TrackersBulkGetParams,
        TrackersListSort, WebhookAction, WebhookDestinationDebugInfo,
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
        trackers_bulk_get::trackers_bulk_get,
        trackers_get::trackers_get,
        trackers_create::trackers_create,
        trackers_update::trackers_update,
        trackers_remove::trackers_remove,
        trackers_bulk_remove::trackers_bulk_remove,
        trackers_list_revisions_batch::trackers_list_revisions_batch,
        trackers_list_revisions::trackers_list_revisions,
        trackers_create_revision::trackers_create_revision,
        trackers_clear_revisions::trackers_clear_revisions,
        trackers_get_revision::trackers_get_revision,
        trackers_remove_revision::trackers_remove_revision,
        trackers_list_execution_logs::trackers_list_execution_logs,
        trackers_list_execution_logs_batch::trackers_list_execution_logs_batch,
        trackers_clear_execution_logs::trackers_clear_execution_logs,
        trackers_clear_all_execution_logs::trackers_clear_all_execution_logs,
        trackers_import_revisions::trackers_import_revisions,
        trackers_debug::trackers_debug,
        trackers_debug_existing::trackers_debug_existing
    ),
    components(schemas(
        ActionDebugInfo,
        DatabaseStatus,
        ActionDestinationDebugInfo,
        ApiRequestDebugInfo,
        ApiTarget,
        ApiTrackerDebugResult,
        AutoParseDebugInfo,
        EmailAction,
        ExtractorEngine,
        Page<Tracker>,
        PageLogEntry,
        PageTarget,
        PageTrackerDebugResult,
        RenderedEmailDebugInfo,
        SchedulerJobConfig,
        SchedulerJobRetryStrategy,
        ScriptDebugInfo,
        ServerLogAction,
        SortOrder,
        Status,
        TargetRequest,
        Tracker,
        TrackerAction,
        TrackerConfig,
        TrackerCreateParams,
        TrackerDataRevision,
        TrackerDataRevisionImportParams,
        TrackerDataRevisionImportResult,
        TrackerDataValue,
        TrackerDebugExistingParams,
        TrackerDebugParams,
        TrackerDebugResult,
        TrackerDebugTargetResult,
        TrackerExecutionLog,
        TrackerExecutionLogPhase,
        TrackerExecutionLogStatus,
        TrackerListExecutionLogsBatchParams,
        TrackerListRevisionsBatchParams,
        TrackerTarget,
        TrackerUpdateParams,
        TrackersBulkGetParams,
        TrackersListSort,
        WebhookAction,
        WebhookDestinationDebugInfo
    ))
)]
pub(super) struct RetrackOpenApi;
