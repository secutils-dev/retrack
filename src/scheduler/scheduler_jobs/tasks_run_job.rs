use crate::{
    api::Api,
    network::DnsResolver,
    scheduler::{
        database_ext::RawSchedulerJobStoredData, job_ext::JobExt, scheduler_job::SchedulerJob,
        CronExt, SchedulerJobMetadata,
    },
};
use anyhow::Context;
use croner::Cron;
use std::{sync::Arc, time::Instant};
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, trace};

/// Defines a maximum number of tasks that can be executed during a single job tick.
const MAX_TASKS_TO_SEND: usize = 100;

/// The job run on a regular interval to check if there are any pending tasks that need to be run.
pub(crate) struct TasksRunJob;
impl TasksRunJob {
    /// Tries to resume existing `TasksRunJob` job.
    pub fn try_resume<DR: DnsResolver>(
        api: Arc<Api<DR>>,
        existing_job_data: RawSchedulerJobStoredData,
    ) -> anyhow::Result<Option<Job>> {
        // If the schedule has changed, remove existing job and create a new one.
        let mut new_job = Self::create(api)?;
        Ok(if new_job.are_schedules_equal(&existing_job_data)? {
            new_job.set_raw_job_data(existing_job_data)?;
            Some(new_job)
        } else {
            None
        })
    }

    /// Creates a new `TasksRunJob` job.
    pub fn create<DR: DnsResolver>(api: Arc<Api<DR>>) -> anyhow::Result<Job> {
        let mut job = Job::new_async(
            Cron::parse_pattern(&api.config.scheduler.tasks_run)
                .with_context(|| {
                    format!(
                        "Cannot parse `tasks_run` schedule: {}",
                        api.config.scheduler.tasks_run
                    )
                })?
                .pattern
                .to_string(),
            move |_, scheduler| {
                let api = api.clone();
                Box::pin(async move {
                    if let Err(err) = Self::execute(api, scheduler).await {
                        error!("Failed to execute tasks run job: {err:?}");
                    }
                })
            },
        )?;

        job.set_job_meta(&SchedulerJobMetadata::new(SchedulerJob::TasksRun))?;

        Ok(job)
    }

    /// Executes a `TasksRunJob` job.
    async fn execute<DR: DnsResolver>(api: Arc<Api<DR>>, _: JobScheduler) -> anyhow::Result<()> {
        let execute_start = Instant::now();
        match api.tasks().execute_pending_tasks(MAX_TASKS_TO_SEND).await {
            Ok(executed_tasks_count) if executed_tasks_count > 0 => {
                info!(
                    "Executed {executed_tasks_count} tasks ({} elapsed).",
                    humantime::format_duration(execute_start.elapsed())
                );
            }
            Ok(_) => {
                trace!(
                    "No pending tasks to execute ({} elapsed).",
                    humantime::format_duration(execute_start.elapsed())
                );
            }
            Err(err) => {
                error!(
                    "Failed to execute pending tasks ({} elapsed): {err:?}",
                    humantime::format_duration(execute_start.elapsed())
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{TasksRunJob, MAX_TASKS_TO_SEND};
    use crate::{
        scheduler::scheduler_job::SchedulerJob,
        tasks::{Email, EmailContent, EmailTaskType, TaskType},
        tests::{
            mock_api_with_config, mock_api_with_network, mock_config, mock_network_with_smtp,
            mock_schedule_in_sec, mock_scheduler, mock_scheduler_job, mock_smtp, mock_smtp_config,
            MockSmtpServer,
        },
    };
    use futures::StreamExt;
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use std::{str::from_utf8, sync::Arc, time::Duration};
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_create_job_with_correct_parameters(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.tasks_run = "1/5 * * * * *".to_string();

        let api = mock_api_with_config(pool, config).await?;

        let mut job = TasksRunJob::create(Arc::new(api))?;
        let job_data = job
            .job_data()
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job))?;
        assert_debug_snapshot!(job_data, @r###"
        (
            0,
            [
                0,
                0,
                0,
            ],
            Some(
                CronJob(
                    CronJob {
                        schedule: "1/5 * * * * *",
                    },
                ),
            ),
        )
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn can_resume_job(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.tasks_run = "0 0 * * * *".to_string();

        let api = mock_api_with_config(pool, config).await?;

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        let job = TasksRunJob::try_resume(
            Arc::new(api),
            mock_scheduler_job(job_id, SchedulerJob::TasksRun, "0 0 * * * *"),
        )?;
        let job_data = job
            .and_then(|mut job| job.job_data().ok())
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job));
        assert_debug_snapshot!(job_data, @r###"
        Some(
            (
                0,
                [
                    0,
                    0,
                    0,
                ],
                Some(
                    CronJob(
                        CronJob {
                            schedule: "0 0 * * * *",
                        },
                    ),
                ),
            ),
        )
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn can_execute_pending_tasks(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let mut api = mock_api_with_network(
            pool,
            mock_network_with_smtp(mock_smtp(mock_smtp_config(
                smtp_server.host.to_string(),
                smtp_server.port,
            ))),
        )
        .await?;
        api.config.scheduler.tasks_run = mock_schedule_in_sec(2);
        let api = Arc::new(api);

        for n in 0..=(MAX_TASKS_TO_SEND as i64) {
            api.tasks()
                .schedule_task(
                    TaskType::Email(EmailTaskType {
                        to: vec!["dev@retrack.dev".to_string()],
                        content: EmailContent::Custom(Email::text(
                            "subject".to_string(),
                            format!("message {n}"),
                        )),
                    }),
                    OffsetDateTime::from_unix_timestamp(946720800 + n)?,
                )
                .await?;
        }

        scheduler.add(TasksRunJob::create(api.clone())?).await?;

        let timestamp = OffsetDateTime::from_unix_timestamp(946730800)?;
        assert_eq!(
            api.db
                .get_tasks_ids(timestamp, 10)
                .collect::<Vec<_>>()
                .await
                .len(),
            101
        );

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        while api
            .db
            .get_tasks_ids(timestamp, 10)
            .collect::<Vec<_>>()
            .await
            .len()
            > 1
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        scheduler.shutdown().await?;

        assert_eq!(
            api.db
                .get_tasks_ids(timestamp, 10)
                .collect::<Vec<_>>()
                .await
                .len(),
            1
        );

        let mails = smtp_server.mails();
        assert_eq!(mails.len(), 100);
        assert_debug_snapshot!(from_utf8(&mails[0].content)?, @r###""From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: dev@retrack.dev\r\nContent-Transfer-Encoding: 7bit\r\n\r\nmessage 0""###);

        Ok(())
    }
}
