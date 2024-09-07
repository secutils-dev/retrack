use crate::tasks::Task;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTask {
    pub id: Uuid,
    pub task_type: Vec<u8>,
    pub scheduled_at: OffsetDateTime,
}

impl TryFrom<RawTask> for Task {
    type Error = anyhow::Error;

    fn try_from(raw_task: RawTask) -> Result<Self, Self::Error> {
        Ok(Task {
            id: raw_task.id,
            task_type: postcard::from_bytes(&raw_task.task_type)?,
            scheduled_at: raw_task.scheduled_at,
        })
    }
}

impl TryFrom<&Task> for RawTask {
    type Error = anyhow::Error;

    fn try_from(task: &Task) -> Result<Self, Self::Error> {
        Ok(RawTask {
            id: task.id,
            task_type: postcard::to_stdvec(&task.task_type)?,
            scheduled_at: task.scheduled_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTask;
    use crate::tasks::{Email, EmailContent, EmailTaskType, Task, TaskType};
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn can_convert_to_task() -> anyhow::Result<()> {
        assert_eq!(
            Task::try_from(RawTask {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: vec![
                    0, 1, 15, 100, 101, 118, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101,
                    118, 0, 4, 115, 117, 98, 106, 10, 101, 109, 97, 105, 108, 32, 116, 101, 120,
                    116, 0, 0
                ],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string()
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        Ok(())
    }

    #[test]
    fn can_convert_to_raw_task() -> anyhow::Result<()> {
        assert_eq!(
            RawTask::try_from(&Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string()
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            RawTask {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: vec![
                    0, 1, 15, 100, 101, 118, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101,
                    118, 0, 4, 115, 117, 98, 106, 10, 101, 109, 97, 105, 108, 32, 116, 101, 120,
                    116, 0, 0
                ],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        Ok(())
    }
}
