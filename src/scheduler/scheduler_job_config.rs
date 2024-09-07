use crate::scheduler::SchedulerJobRetryStrategy;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Represents a job configuration that can be scheduled.
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerJobConfig {
    /// Defines a schedule for the job.
    pub schedule: String,
    /// Defines a retry strategy for the job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_strategy: Option<SchedulerJobRetryStrategy>,
}
