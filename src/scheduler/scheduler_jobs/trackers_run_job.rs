use crate::{
    api::Api,
    error::Error as RetrackError,
    network::{DnsResolver, EmailTransport, EmailTransportError},
    scheduler::{
        database_ext::RawSchedulerJobStoredData, job_ext::JobExt, scheduler_job::SchedulerJob,
        CronExt,
    },
    tasks::{EmailContent, EmailTaskType, EmailTemplate, TaskType},
    trackers::Tracker,
};
use anyhow::Context;
use croner::Cron;
use futures::{pin_mut, StreamExt};
use std::{sync::Arc, time::Instant};
use time::OffsetDateTime;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, warn};
use uuid::Uuid;

/// The job executes every minute by default to check if there are any trackers to run.
pub(crate) struct TrackersRunJob;
impl TrackersRunJob {
    /// Tries to resume existing `TrackersRun` job.
    pub async fn try_resume<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
        existing_job_data: RawSchedulerJobStoredData,
    ) -> anyhow::Result<Option<Job>>
    where
        ET::Error: EmailTransportError,
    {
        // If the schedule has changed, remove existing job and create a new one.
        let mut new_job = Self::create(api).await?;
        Ok(if new_job.are_schedules_equal(&existing_job_data)? {
            new_job.set_raw_job_data(existing_job_data)?;
            Some(new_job)
        } else {
            None
        })
    }

    /// Creates a new `TrackersRun` job.
    pub async fn create<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
    ) -> anyhow::Result<Job>
    where
        ET::Error: EmailTransportError,
    {
        let mut job = Job::new_async(
            Cron::parse_pattern(&api.config.scheduler.trackers_run)
                .with_context(|| {
                    format!(
                        "Cannot parse `trackers_run` schedule: {}",
                        api.config.scheduler.trackers_run
                    )
                })?
                .pattern
                .to_string(),
            move |_, scheduler| {
                let api = api.clone();
                Box::pin(async move {
                    if let Err(err) = Self::execute(api, scheduler).await {
                        error!("Failed to execute trackers run job: {err:?}");
                    }
                })
            },
        )?;

        job.set_job_type(SchedulerJob::TrackersRun)?;

        Ok(job)
    }

    /// Executes a `TrackersRun` job.
    async fn execute<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
        scheduler: JobScheduler,
    ) -> anyhow::Result<()>
    where
        ET::Error: EmailTransportError,
    {
        Self::run(api, scheduler).await?;

        Ok(())
    }

    async fn run<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
        scheduler: JobScheduler,
    ) -> anyhow::Result<()>
    where
        ET::Error: EmailTransportError,
    {
        // Fetch all trackers jobs that are pending processing.
        let trackers = api.trackers();
        let pending_trackers = trackers.get_trackers_to_run();
        pin_mut!(pending_trackers);

        while let Some(tracker) = pending_trackers.next().await {
            let Some((tracker, job_id)) =
                Self::validate_tracker(&api, &scheduler, tracker?).await?
            else {
                continue;
            };

            // Try to create a new revision. If a revision is returned that means that tracker
            // detected changes.
            let run_start = Instant::now();
            let new_revision = match api
                .trackers()
                .create_tracker_data_revision(tracker.id)
                .await
            {
                Ok(new_revision) => new_revision,
                Err(err) => {
                    let execution_time = run_start.elapsed();
                    error!(
                        tracker.id = %tracker.id,
                        tracker.name = tracker.name,
                        metrics.job_execution_time = execution_time.as_nanos() as u64,
                        "Failed to create tracker data revision: {err:?}"
                    );

                    // Check if the tracker has a retry strategy.
                    let retry_strategy = tracker
                        .config
                        .job
                        .as_ref()
                        .and_then(|job_config| job_config.retry_strategy);
                    let retry_state = if let Some(retry_strategy) = retry_strategy {
                        api.scheduler()
                            .schedule_retry(job_id, &retry_strategy)
                            .await?
                    } else {
                        None
                    };

                    if let Some(retry) = retry_state {
                        warn!(
                            tracker.id = %tracker.id,
                            tracker.name = tracker.name,
                            metrics.job_retries = retry.attempts,
                            "Scheduled a retry to create tracker data revision at {}.",
                            retry.next_at,
                        );
                    } else {
                        // Re-schedule the job.
                        api.db.reset_scheduler_job_state(job_id, false).await?;

                        // Report the error.
                        Self::report_error(&api, tracker, err).await;
                    }

                    continue;
                }
            };

            let execution_time = run_start.elapsed();
            if let Some(revision) = new_revision {
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    metrics.job_execution_time = execution_time.as_nanos() as u64,
                    metrics.tracker_data_size = revision.data.size(),
                    "Successfully checked tracker data — changes detected, and a new data revision has been created."
                );
            } else {
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    metrics.job_execution_time = execution_time.as_nanos() as u64,
                    "Successfully checked tracker data — no changes detected since the last check."
                );
            }

            api.db.reset_scheduler_job_state(job_id, false).await?;
        }

        Ok(())
    }

    async fn validate_tracker<DR: DnsResolver, ET: EmailTransport>(
        api: &Api<DR, ET>,
        scheduler: &JobScheduler,
        tracker: Tracker,
    ) -> anyhow::Result<Option<(Tracker, Uuid)>>
    where
        ET::Error: EmailTransportError,
    {
        let Some(job_id) = tracker.job_id else {
            error!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Could not find a job for a pending tracker, skipping."
            );
            return Ok(None);
        };

        if !tracker.enabled || tracker.config.revisions == 0 || tracker.config.job.is_none() {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %job_id,
                "Found a pending tracker that is disabled or doesn't support tracking, the job will be removed."
            );

            scheduler.remove(&job_id).await?;
            api.trackers().update_tracker_job(tracker.id, None).await?;
            return Ok(None);
        }

        Ok(Some((tracker, job_id)))
    }

    async fn report_error<DR: DnsResolver, ET: EmailTransport>(
        api: &Api<DR, ET>,
        tracker: Tracker,
        error: anyhow::Error,
    ) where
        ET::Error: EmailTransportError,
    {
        let Some(ref smtp_config) = api.config.smtp else {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Failed to report failed tracker data check: SMTP configuration is missing."
            );
            return;
        };

        let Some(ref catch_all_recipient) = smtp_config.catch_all else {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Failed to report failed tracker data check: catch-all recipient is missing."
            );
            return;
        };

        let email_task = TaskType::Email(EmailTaskType {
            to: vec![catch_all_recipient.recipient.clone()],
            content: EmailContent::Template(EmailTemplate::TrackerChanges {
                tracker_name: tracker.name.clone(),
                content: Err(error
                    .downcast::<RetrackError>()
                    .map(|err| format!("{err}"))
                    .unwrap_or_else(|_| "Unknown error".to_string())),
            }),
        });

        let tasks_schedule_result = api
            .tasks()
            .schedule_task(email_task, OffsetDateTime::now_utc())
            .await;
        if let Err(err) = tasks_schedule_result {
            error!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Failed to report failed tracker data check: {err:?}."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TrackersRunJob;
    use crate::{
        config::SmtpConfig,
        scheduler::{
            scheduler_job::SchedulerJob, scheduler_jobs::TrackersTriggerJob, SchedulerJobConfig,
            SchedulerJobRetryStrategy,
        },
        tests::{
            mock_api_with_config, mock_config, mock_get_scheduler_job, mock_schedule_in_sec,
            mock_schedule_in_secs, mock_scheduler, mock_scheduler_job, SmtpCatchAllConfig,
            WebScraperContentRequest, WebScraperErrorResponse,
        },
        trackers::{
            EmailAction, PageTarget, Tracker, TrackerAction, TrackerConfig, TrackerCreateParams,
            TrackerDataRevision, TrackerDataValue, TrackerTarget,
        },
    };
    use futures::StreamExt;
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use regex::Regex;
    use serde_json::json;
    use sqlx::PgPool;
    use std::{default::Default, ops::Add, sync::Arc, time::Duration};
    use time::OffsetDateTime;
    use url::Url;
    use uuid::{uuid, Uuid};

    #[sqlx::test]
    async fn can_create_job_with_correct_parameters(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = "1/5 * * * * *".to_string();

        let api = mock_api_with_config(pool, config).await?;

        let mut job = TrackersRunJob::create(Arc::new(api)).await?;
        let job_data = job
            .job_data()
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job))?;
        assert_debug_snapshot!(job_data, @r###"
        (
            0,
            [
                2,
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
        config.scheduler.trackers_run = "0 0 * * * *".to_string();

        let api = mock_api_with_config(pool, config).await?;

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        let job = TrackersRunJob::try_resume(
            Arc::new(api),
            mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 0 * * * *"),
        )
        .await?;
        let job_data = job
            .and_then(|mut job| job.job_data().ok())
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job));
        assert_debug_snapshot!(job_data, @r###"
        Some(
            (
                3,
                [
                    2,
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
    async fn remove_pending_trackers_jobs_if_zero_revisions(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = mock_schedule_in_sec(2);

        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create tracker.
        let tracker_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), mock_schedule_in_sec(1)).await?)
            .await?;
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParams::new("tracker")
                    .with_config(TrackerConfig {
                        revisions: 0,
                        ..Default::default()
                    })
                    .with_schedule("0 0 * * * *"),
            )
            .await?;
        api.trackers()
            .update_tracker_job(tracker.id, Some(tracker_job_id))
            .await?;

        // Schedule run job
        scheduler
            .add(TrackersRunJob::create(api.clone()).await?)
            .await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        let trackers = api.trackers();
        while trackers
            .get_tracker_by_job_id(tracker_job_id)
            .await?
            .is_some()
            || trackers
                .get_tracker_by_job_id(tracker_job_id)
                .await?
                .is_some()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;

        Ok(())
    }

    #[sqlx::test]
    async fn remove_pending_trackers_jobs_if_disabled(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = mock_schedule_in_sec(2);

        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create tracker.
        let tracker_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), mock_schedule_in_sec(1)).await?)
            .await?;
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParams::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .disable(),
            )
            .await?;
        api.trackers()
            .update_tracker_job(tracker.id, Some(tracker_job_id))
            .await?;

        // Schedule run job
        scheduler
            .add(TrackersRunJob::create(api.clone()).await?)
            .await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        let trackers = api.trackers();
        while trackers
            .get_tracker_by_job_id(tracker_job_id)
            .await?
            .is_some()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;

        Ok(())
    }

    #[sqlx::test]
    async fn can_run(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = mock_schedule_in_sec(3);

        let server = MockServer::start();
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create tracker and tracker job.
        // Make sure that the tracker is only run once during a single minute (2 seconds after the
        // current second).
        let tracker_schedule = mock_schedule_in_sec(1);
        let trigger_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), tracker_schedule.clone()).await?)
            .await?;
        let tracker = Tracker {
            id: Uuid::now_v7(),
            name: "tracker".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                user_agent: Some("Retrack/1.0.0".parse()?),
                ignore_https_errors: true,
            }),
            config: TrackerConfig {
                revisions: 1,
                timeout: None,
                headers: Some(
                    [("cookie".to_string(), "my-cookie".to_string())]
                        .into_iter()
                        .collect(),
                ),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: None
                }),
            },
            actions: vec![TrackerAction::ServerLog],
            tags: vec![],
            job_id: Some(trigger_job_id),
            // Preserve timestamp only up to seconds.
            created_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
            updated_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
        };

        // Insert tracker directly to DB to bypass schedule validation.
        api.db.trackers().insert_tracker(&tracker).await?;

        // Schedule run job
        scheduler
            .add(TrackersRunJob::create(api.clone()).await?)
            .await?;

        // Create a mock
        let content = TrackerDataValue::new(json!("some-content"));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
                        .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value());
        });

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        let trackers = api.trackers();
        while trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;

        content_mock.assert();

        // Check that content was saved.
        assert_eq!(
            api.trackers()
                .get_tracker_data(tracker.id, Default::default())
                .await?
                .into_iter()
                .map(|rev| rev.data)
                .collect::<Vec<_>>(),
            vec![content]
        );

        // Check that the tracker job was marked as NOT stopped.
        let trigger_job = mock_get_scheduler_job(&api.db, trigger_job_id).await?;
        assert_eq!(
            trigger_job.map(|job| (job.id, job.stopped)),
            Some((trigger_job_id, Some(false)))
        );

        Ok(())
    }

    #[sqlx::test]
    async fn schedules_task_when_content_change(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = mock_schedule_in_sec(3);

        let server = MockServer::start();
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let mut scheduler = mock_scheduler(&pool).await?;
        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Make sure that the tracker is only run once during a single minute (2 seconds after the
        // current second).
        let tracker_schedule = mock_schedule_in_sec(1);

        // Create tracker and tracker job.
        let trigger_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), tracker_schedule.clone()).await?)
            .await?;
        let tracker = Tracker {
            id: Uuid::now_v7(),
            name: "tracker-one".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                user_agent: Some("Retrack/1.0.0".parse()?),
                ignore_https_errors: true,
            }),
            config: TrackerConfig {
                revisions: 2,
                timeout: Some(Duration::from_secs(2)),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: None
                }),
            },
            actions: vec![
                TrackerAction::ServerLog,
                TrackerAction::Email(EmailAction { to: vec!["dev@retrack.dev".to_string()] })
            ],
            tags: vec![],
            job_id: Some(trigger_job_id),
            // Preserve timestamp only up to seconds.
            created_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
            updated_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
        };

        // Insert tracker directly to DB to bypass schedule validation.
        api.db.trackers().insert_tracker(&tracker).await?;
        api.db
            .trackers()
            .insert_tracker_data_revision(&TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: tracker.id,
                created_at: OffsetDateTime::from_unix_timestamp(946720700)?,
                data: TrackerDataValue::new(json!("some-content")),
            })
            .await?;

        // Schedule run job
        scheduler
            .add(TrackersRunJob::create(api.clone()).await?)
            .await?;

        // Create a mock
        let content = json!("other-content");
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content(&json!("some-content")),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content);
        });

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        while api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;

        content_mock.assert();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let task = api.db.get_task(tasks_ids.remove(0)?).await?;
        assert_debug_snapshot!(task.map(|task| task.task_type), @r###"
        Some(
            Email(
                EmailTaskType {
                    to: [
                        "dev@retrack.dev",
                    ],
                    content: Template(
                        TrackerChanges {
                            tracker_name: "tracker-one",
                            content: Ok(
                                "\"other-content\"",
                            ),
                        },
                    ),
                },
            ),
        )
        "###);

        assert_eq!(
            api.trackers()
                .get_tracker_data(tracker.id, Default::default())
                .await?
                .len(),
            2
        );
        assert!(!mock_get_scheduler_job(&api.db, trigger_job_id)
            .await?
            .and_then(|job| job.stopped)
            .unwrap_or_default());

        Ok(())
    }

    #[sqlx::test]
    async fn schedules_task_when_content_change_check_fails(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = mock_schedule_in_sec(3);
        config.smtp = config.smtp.map(|config| SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "dev@retrack.dev".to_string(),
                text_matcher: Regex::new(r"alpha").unwrap(),
            }),
            ..config
        });

        let server = MockServer::start();
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Make sure that the tracker is only run once during a single minute (2 seconds after the
        // current second).
        let tracker_schedule = mock_schedule_in_sec(1);

        // Create tracker and tracker job.
        let trigger_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), tracker_schedule.clone()).await?)
            .await?;
        let tracker = Tracker {
            id: Uuid::now_v7(),
            name: "tracker-one".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                user_agent: Some("Retrack/1.0.0".parse()?),
                ignore_https_errors: true,
            }),
            config: TrackerConfig {
                revisions: 2,
                timeout: Some(Duration::from_secs(2)),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: None
                }),
            },
            tags: vec![],
            actions: vec![TrackerAction::ServerLog],
            job_id: Some(trigger_job_id),
            // Preserve timestamp only up to seconds.
            created_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
            updated_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
        };

        // Insert tracker directly to DB to bypass schedule validation.
        api.db.trackers().insert_tracker(&tracker).await?;
        api.db
            .trackers()
            .insert_tracker_data_revision(&TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: tracker.id,
                created_at: OffsetDateTime::from_unix_timestamp(946720700)?,
                data: TrackerDataValue::new(json!("some-content")),
            })
            .await?;

        // Schedule run job
        scheduler
            .add(TrackersRunJob::create(api.clone()).await?)
            .await?;

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content(&json!("some-content")),
                    )
                    .unwrap(),
                );
            then.status(400)
                .header("Content-Type", "application/json")
                .json_body_obj(&WebScraperErrorResponse {
                    message: "some client-error".to_string(),
                });
        });

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        while api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;

        content_mock.assert();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let task = api.db.get_task(tasks_ids.remove(0)?).await?;
        assert_debug_snapshot!(task.map(|task| task.task_type), @r###"
        Some(
            Email(
                EmailTaskType {
                    to: [
                        "dev@retrack.dev",
                    ],
                    content: Template(
                        TrackerChanges {
                            tracker_name: "tracker-one",
                            content: Err(
                                "some client-error",
                            ),
                        },
                    ),
                },
            ),
        )
        "###);

        assert_eq!(
            api.trackers()
                .get_tracker_data(tracker.id, Default::default())
                .await?
                .len(),
            1
        );
        assert!(!mock_get_scheduler_job(&api.db, trigger_job_id)
            .await?
            .and_then(|job| job.stopped)
            .unwrap_or_default());

        Ok(())
    }

    #[sqlx::test]
    async fn retries_when_content_change_check_fails(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = mock_schedule_in_secs(&[3, 6]);
        config.smtp = config.smtp.map(|config| SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "dev@retrack.dev".to_string(),
                text_matcher: Regex::new(r"alpha").unwrap(),
            }),
            ..config
        });

        let server = MockServer::start();
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let mut scheduler = mock_scheduler(&pool).await?;
        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Make sure that the tracker is only run once during a single minute (2 seconds after the
        // current second).
        let tracker_schedule = mock_schedule_in_sec(1);

        // Create tracker and tracker job.
        let trigger_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), tracker_schedule.clone()).await?)
            .await?;
        let tracker = Tracker {
            id: Uuid::now_v7(),
            name: "tracker-one".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                user_agent: Some("Retrack/1.0.0".to_string()),
                ignore_https_errors: true,
            }),
            config: TrackerConfig {
                revisions: 2,
                timeout: Some(Duration::from_secs(2)),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(1),
                        max_attempts: 1,
                    })
                }),
            },
            actions: vec![TrackerAction::ServerLog],
            tags: vec![],

            job_id: Some(trigger_job_id),
            // Preserve timestamp only up to seconds.
            created_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
            updated_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
        };

        // Insert tracker directly to DB to bypass schedule validation.
        api.db.trackers().insert_tracker(&tracker).await?;
        api.db
            .trackers()
            .insert_tracker_data_revision(&TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: tracker.id,
                created_at: OffsetDateTime::from_unix_timestamp(946720700)?,
                data: TrackerDataValue::new(json!("some-content")),
            })
            .await?;

        // Schedule run job
        scheduler
            .add(TrackersRunJob::create(api.clone()).await?)
            .await?;

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content(&json!("some-content")),
                    )
                    .unwrap(),
                );
            then.status(400)
                .header("Content-Type", "application/json")
                .json_body_obj(&WebScraperErrorResponse {
                    message: "some client-error".to_string(),
                });
        });

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        while api
            .db
            .get_scheduler_job_meta(trigger_job_id)
            .await?
            .unwrap()
            .retry
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        content_mock.assert();

        let tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        assert_eq!(
            api.trackers()
                .get_tracker_data(tracker.id, Default::default())
                .await?
                .len(),
            1
        );
        assert!(mock_get_scheduler_job(&api.db, trigger_job_id)
            .await?
            .and_then(|job| job.stopped)
            .unwrap_or_default());

        while api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;

        let mut tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let task = api.db.get_task(tasks_ids.remove(0)?).await?;
        assert_debug_snapshot!(task.map(|task| task.task_type), @r###"
        Some(
            Email(
                EmailTaskType {
                    to: [
                        "dev@retrack.dev",
                    ],
                    content: Template(
                        TrackerChanges {
                            tracker_name: "tracker-one",
                            content: Err(
                                "some client-error",
                            ),
                        },
                    ),
                },
            ),
        )
        "###);

        assert_eq!(
            api.trackers()
                .get_tracker_data(tracker.id, Default::default())
                .await?
                .len(),
            1
        );
        assert!(!mock_get_scheduler_job(&api.db, trigger_job_id)
            .await?
            .and_then(|job| job.stopped)
            .unwrap_or_default());

        Ok(())
    }

    #[sqlx::test]
    async fn retries_when_content_change_check_fails_until_succeeds(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_run = mock_schedule_in_secs(&[3, 6]);

        let server = MockServer::start();
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let mut scheduler = mock_scheduler(&pool).await?;

        // Make sure that the tracker is only run once during a single minute (2 seconds after the
        // current second).
        let tracker_schedule = mock_schedule_in_sec(1);

        // Create tracker and tracker job.
        let api = Arc::new(mock_api_with_config(pool, config).await?);
        let trigger_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), tracker_schedule.clone()).await?)
            .await?;
        let tracker = Tracker {
            id: Uuid::now_v7(),
            name: "tracker-one".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                user_agent: Some("Retrack/1.0.0".to_string()),
                ignore_https_errors: true,
            }),
            config: TrackerConfig {
                revisions: 2,
                timeout: Some(Duration::from_secs(2)),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(1),
                        max_attempts: 1,
                    })
                }),
            },
            tags: vec![],
            actions: vec![
                TrackerAction::ServerLog,
                TrackerAction::Email(EmailAction { to: vec!["dev@retrack.dev".to_string()] })
            ],
            job_id: Some(trigger_job_id),
            // Preserve timestamp only up to seconds.
            created_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
            updated_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
        };

        // Insert tracker directly to DB to bypass schedule validation.
        api.db.trackers().insert_tracker(&tracker).await?;
        api.db
            .trackers()
            .insert_tracker_data_revision(&TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: tracker.id,
                created_at: OffsetDateTime::from_unix_timestamp(946720700)?,
                data: TrackerDataValue::new(json!("some-content")),
            })
            .await?;

        // Schedule run job
        scheduler
            .add(TrackersRunJob::create(api.clone()).await?)
            .await?;

        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content(&json!("some-content")),
                    )
                    .unwrap(),
                );
            then.status(400)
                .header("Content-Type", "application/json")
                .json_body_obj(&WebScraperErrorResponse {
                    message: "some client-error".to_string(),
                });
        });

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        while api
            .db
            .get_scheduler_job_meta(trigger_job_id)
            .await?
            .unwrap()
            .retry
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        content_mock.assert();
        content_mock.delete();

        let tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        assert_eq!(
            api.trackers()
                .get_tracker_data(tracker.id, Default::default())
                .await?
                .len(),
            1
        );
        assert!(mock_get_scheduler_job(&api.db, trigger_job_id)
            .await?
            .and_then(|job| job.stopped)
            .unwrap_or_default());

        // Create a mock
        let content = json!("other-content");
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content(&json!("some-content")),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content);
        });

        while api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;

        content_mock.assert();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let task = api.db.get_task(tasks_ids.remove(0)?).await?;
        assert_debug_snapshot!(task.map(|task| task.task_type), @r###"
        Some(
            Email(
                EmailTaskType {
                    to: [
                        "dev@retrack.dev",
                    ],
                    content: Template(
                        TrackerChanges {
                            tracker_name: "tracker-one",
                            content: Ok(
                                "\"other-content\"",
                            ),
                        },
                    ),
                },
            ),
        )
        "###);

        assert_eq!(
            api.trackers()
                .get_tracker_data(tracker.id, Default::default())
                .await?
                .len(),
            2
        );
        assert!(!mock_get_scheduler_job(&api.db, trigger_job_id)
            .await?
            .and_then(|job| job.stopped)
            .unwrap_or_default());

        Ok(())
    }
}
