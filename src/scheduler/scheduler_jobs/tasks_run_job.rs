use crate::{
    api::Api,
    network::DnsResolver,
    scheduler::{
        CronExt, SchedulerJobMetadata, database_ext::RawSchedulerJobStoredData, job_ext::JobExt,
        scheduler_job::SchedulerJob,
    },
};
use anyhow::Context;
use croner::Cron;
use std::{sync::Arc, time::Instant};
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{debug, error, info, trace};
use uuid::Uuid;

/// Defines a maximum number of tasks that can be executed during a single job tick.
const MAX_TASKS_TO_SEND: usize = 100;

/// The job run on a regular interval to check if there are any pending tasks that need to be run.
pub(crate) struct TasksRunJob;
impl TasksRunJob {
    /// Tries to resume the existing `TasksRunJob` job.
    pub fn try_resume<DR: DnsResolver>(
        api: Arc<Api<DR>>,
        existing_job_data: RawSchedulerJobStoredData,
    ) -> anyhow::Result<Option<Job>> {
        let Some(Ok(job_meta)) = existing_job_data
            .extra
            .as_ref()
            .map(|extra| SchedulerJobMetadata::try_from(extra.as_slice()))
        else {
            error!(
                job.id = %existing_job_data.id,
                "The job doesn't have metadata and will be removed."
            );
            return Ok(None);
        };

        // If the job was marked as running, remove it.
        if job_meta.is_running {
            error!(
                job.id = %existing_job_data.id,
                "The job was marked as running and will be removed."
            );
            return Ok(None);
        }

        // If the schedule has changed, remove the existing job and create a new one.
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
            move |job_id, scheduler| {
                let api = api.clone();
                Box::pin(async move {
                    debug!(job.id = %job_id, "Running job.");
                    if let Err(err) = Self::execute(job_id, api, scheduler).await {
                        error!(job.id = %job_id, "Job failed with unexpected error: {err:?}.");
                    } else {
                        debug!(job.id = %job_id, "Finished running job.");
                    }
                })
            },
        )?;

        job.set_job_meta(&Self::create_job_meta())?;

        Ok(job)
    }

    /// Executes a `TasksRunJob` job.
    async fn execute<DR: DnsResolver>(
        job_id: Uuid,
        api: Arc<Api<DR>>,
        job_scheduler: JobScheduler,
    ) -> anyhow::Result<()> {
        let run_start = Instant::now();
        let scheduler = api.scheduler();
        let Some(mut job_meta) = scheduler.get_job_meta(job_id).await? else {
            error!(
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "The job doesn't have metadata and will be removed."
            );
            job_scheduler.remove(&job_id).await?;
            return Ok(());
        };

        // The job shouldn't be yet running. If not, skip it.
        if job_meta.is_running {
            debug!(
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "The job is already running and will be skipped."
            );
            return Ok(());
        }

        // Now, mark the job as running, preserving the rest of the metadata.
        scheduler
            .set_job_meta(job_id, job_meta.set_running())
            .await?;
        match api.tasks().execute_pending_tasks(MAX_TASKS_TO_SEND).await {
            Ok(executed_tasks_count) if executed_tasks_count > 0 => {
                info!(
                    "Executed {executed_tasks_count} tasks ({} elapsed).",
                    humantime::format_duration(run_start.elapsed())
                );
            }
            Ok(_) => {
                trace!(
                    "No pending tasks to execute ({} elapsed).",
                    humantime::format_duration(run_start.elapsed())
                );
            }
            Err(err) => {
                error!(
                    "Failed to execute pending tasks ({} elapsed): {err:?}",
                    humantime::format_duration(run_start.elapsed())
                );
            }
        }

        // Reset the job metadata (is_running will be set to false).
        scheduler
            .set_job_meta(job_id, &Self::create_job_meta())
            .await?;

        Ok(())
    }

    /// Creates a new `TasksRun` job metadata.
    fn create_job_meta() -> SchedulerJobMetadata {
        SchedulerJobMetadata::new(SchedulerJob::TasksRun)
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TASKS_TO_SEND, TasksRunJob};
    use crate::{
        scheduler::{SchedulerJobMetadata, scheduler_job::SchedulerJob},
        tasks::{Email, EmailContent, EmailTaskType, HttpTaskType, TaskType},
        tests::{
            MockSmtpServer, mock_api, mock_api_with_config, mock_api_with_network, mock_config,
            mock_network_with_smtp, mock_schedule_in_sec, mock_scheduler, mock_scheduler_job,
            mock_smtp, mock_smtp_config, mock_upsert_scheduler_job,
        },
    };
    use futures::StreamExt;
    use http::Method;
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use serde_json::json;
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
    async fn removes_job_if_does_not_have_meta(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");
        let mut mock_job = mock_scheduler_job(job_id, SchedulerJob::TasksRun, "0 0 * * * *");
        mock_job.extra = None;
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TasksRunJob::try_resume(api.clone(), mock_job)?;
        assert!(job.is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn removes_job_if_it_was_running(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");
        let mut mock_job = mock_scheduler_job(job_id, SchedulerJob::TasksRun, "0 0 * * * *");
        mock_job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TasksRun).set_running(),
        )?);
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TasksRunJob::try_resume(api.clone(), mock_job)?;
        assert!(job.is_none());

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
                    vec![],
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

        // Start the scheduler and wait for a few seconds, then stop it.
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

    #[sqlx::test]
    async fn properly_sets_is_running_flag(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let mut api = mock_api(pool).await?;
        api.config.scheduler.tasks_run = mock_schedule_in_sec(2);
        let api = Arc::new(api);

        let server = MockServer::start();
        let server_handler_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/some/execute");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&json!({ "ok": true }))
                .delay(Duration::from_secs(3));
        });

        let tasks_api = api.tasks();
        let task = tasks_api
            .schedule_task(
                TaskType::Http(HttpTaskType {
                    url: format!("{}/api/some/execute", server.base_url()).parse()?,
                    method: Method::POST,
                    headers: None,
                    body: None,
                }),
                vec![],
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;

        let job_id = scheduler.add(TasksRunJob::create(api.clone())?).await?;

        // Job should not be running, by default.
        let scheduler_api = api.scheduler();
        assert!(
            !scheduler_api
                .get_job_meta(job_id)
                .await?
                .unwrap()
                .is_running
        );

        // Start the scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        // First, wait until the job is running.
        let mut is_running = false;
        while !is_running {
            is_running = scheduler_api
                .get_job_meta(job_id)
                .await?
                .unwrap()
                .is_running;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Then, wait until the job is stopped.
        while is_running {
            is_running = scheduler_api
                .get_job_meta(job_id)
                .await?
                .unwrap()
                .is_running;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;
        server_handler_mock.assert();

        assert!(api.db.get_task(task.id).await?.is_none());

        Ok(())
    }
}
