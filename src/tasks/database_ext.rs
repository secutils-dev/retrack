mod raw_task;

use crate::{
    database::Database,
    tasks::{database_ext::raw_task::RawTask, Task},
};
use async_stream::try_stream;
use futures::Stream;
use sqlx::{query, query_as};
use time::OffsetDateTime;
use uuid::Uuid;

/// Extends primary database with the tasks-related methods.
impl Database {
    /// Retrieves task from the database using ID.
    pub async fn get_task(&self, id: Uuid) -> anyhow::Result<Option<Task>> {
        query_as!(RawTask, r#"SELECT * FROM tasks WHERE id = $1"#, id)
            .fetch_optional(&self.pool)
            .await?
            .map(Task::try_from)
            .transpose()
    }

    /// Inserts a new task to the database.
    pub async fn insert_task(&self, task: &Task) -> anyhow::Result<()> {
        let raw_task = RawTask::try_from(task)?;
        query!(
            r#"INSERT INTO tasks (id, task_type, scheduled_at) VALUES ($1, $2, $3)"#,
            raw_task.id,
            raw_task.task_type,
            raw_task.scheduled_at
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Removes task from the database using ID.
    pub async fn remove_task(&self, id: Uuid) -> anyhow::Result<()> {
        query!(r#"DELETE FROM tasks WHERE id = $1"#, id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Retrieves a list of tasks IDs that are scheduled at or before specified date.
    pub fn get_tasks_ids(
        &self,
        scheduled_before_or_at: OffsetDateTime,
        page_size: usize,
    ) -> impl Stream<Item = anyhow::Result<Uuid>> + '_ {
        let page_limit = page_size as i64;
        try_stream! {
            let mut last_id = Uuid::nil();
            let mut conn = self.pool.acquire().await?;
            loop {
                 let raw_tasks_ids = query!(
                    r#"SELECT id FROM tasks WHERE scheduled_at <= $1 AND id > $2 ORDER BY scheduled_at, id LIMIT $3;"#,
                    scheduled_before_or_at,
                    last_id,
                    page_limit
                ).fetch_all(&mut *conn).await?;

                let is_last_page = raw_tasks_ids.len() < page_size;
                for raw_task_id in raw_tasks_ids {
                    last_id = raw_task_id.id;
                    yield raw_task_id.id;
                }

                if is_last_page {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        database::Database,
        tasks::{Email, EmailContent, EmailTaskType, Task, TaskType},
    };
    use futures::StreamExt;
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use time::OffsetDateTime;
    use uuid::{uuid, Uuid};

    #[sqlx::test]
    async fn can_add_and_retrieve_tasks(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .is_none());

        let tasks = vec![
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string(),
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj #2".to_string(),
                        "email text #2".to_string(),
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        for task in tasks {
            db.insert_task(&task).await?;
        }

        assert_debug_snapshot!(db.get_task(uuid!("00000000-0000-0000-0000-000000000001")).await?, @r###"
        Some(
            Task {
                id: 00000000-0000-0000-0000-000000000001,
                task_type: Email(
                    EmailTaskType {
                        to: [
                            "dev@retrack.dev",
                        ],
                        content: Custom(
                            Email {
                                subject: "subj",
                                text: "email text",
                                html: None,
                                attachments: None,
                            },
                        ),
                    },
                ),
                scheduled_at: 2000-01-01 10:00:00.0 +00:00:00,
            },
        )
        "###);
        assert_debug_snapshot!(db.get_task(uuid!("00000000-0000-0000-0000-000000000002")).await?, @r###"
        Some(
            Task {
                id: 00000000-0000-0000-0000-000000000002,
                task_type: Email(
                    EmailTaskType {
                        to: [
                            "dev@retrack.dev",
                        ],
                        content: Custom(
                            Email {
                                subject: "subj #2",
                                text: "email text #2",
                                html: None,
                                attachments: None,
                            },
                        ),
                    },
                ),
                scheduled_at: 2000-01-01 10:00:00.0 +00:00:00,
            },
        )
        "###);
        assert_debug_snapshot!(db.get_task(uuid!("00000000-0000-0000-0000-000000000003")).await?, @"None");

        Ok(())
    }

    #[sqlx::test]
    async fn can_remove_tasks(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tasks = vec![
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string(),
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj #2".to_string(),
                        "email text #2".to_string(),
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        for task in tasks {
            db.insert_task(&task).await?;
        }

        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .is_some());
        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?
            .is_some());

        db.remove_task(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?;

        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .is_none());
        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?
            .is_some());

        db.remove_task(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?;

        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .is_none());
        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?
            .is_none());

        assert!(db
            .get_task(uuid!("00000000-0000-0000-0000-000000000003"))
            .await?
            .is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn can_get_tasks_ids(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let scheduled_before_or_at = OffsetDateTime::from_unix_timestamp(946720710)?;

        let tasks = db.get_tasks_ids(scheduled_before_or_at, 2);
        assert_eq!(tasks.collect::<Vec<_>>().await.len(), 0);

        for n in 0..=19 {
            db.insert_task(&Task {
                id: Uuid::parse_str(&format!("00000000-0000-0000-0000-0000000000{:02X}", n + 1))?,
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        format!("subj {n}"),
                        format!("email text {n}"),
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720700 + n)?,
            })
            .await?;
        }

        let tasks_ids = db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 11);

        assert_debug_snapshot!(tasks_ids
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?, @r###"
        [
            00000000-0000-0000-0000-000000000001,
            00000000-0000-0000-0000-000000000002,
            00000000-0000-0000-0000-000000000003,
            00000000-0000-0000-0000-000000000004,
            00000000-0000-0000-0000-000000000005,
            00000000-0000-0000-0000-000000000006,
            00000000-0000-0000-0000-000000000007,
            00000000-0000-0000-0000-000000000008,
            00000000-0000-0000-0000-000000000009,
            00000000-0000-0000-0000-00000000000a,
            00000000-0000-0000-0000-00000000000b,
        ]
        "###);

        Ok(())
    }
}
