use crate::tasks::TaskType;
use time::OffsetDateTime;
use uuid::Uuid;

/// Defines a task.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Task {
    /// Unique id of the task.
    pub id: Uuid,
    /// The type of the task.
    pub task_type: TaskType,
    /// Arbitrary tags associated with the task.
    pub tags: Vec<String>,
    /// The time at which the task is scheduled to be executed, in UTC.
    pub scheduled_at: OffsetDateTime,
    /// The number of times the task has been retried.
    pub retry_attempt: Option<u32>,
}
