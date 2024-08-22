use crate::{
    api::Api,
    error::Error as RetrackError,
    network::{DnsResolver, EmailTransport, EmailTransportError},
    notifications::{NotificationContent, NotificationContentTemplate, NotificationDestination},
    scheduler::{
        database_ext::RawSchedulerJobStoredData, job_ext::JobExt, scheduler_job::SchedulerJob,
    },
    trackers::Tracker,
};
use futures::{pin_mut, StreamExt};
use std::{sync::Arc, time::Instant};
use time::OffsetDateTime;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, warn};
use uuid::Uuid;

/// The job executes every minute by default to check if there are any trackers to fetch for.
pub(crate) struct TrackersFetchJob;
impl TrackersFetchJob {
    /// Tries to resume existing `TrackersFetch` job.
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

    /// Creates a new `WebPageTrackersFetch` job.
    pub async fn create<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
    ) -> anyhow::Result<Job>
    where
        ET::Error: EmailTransportError,
    {
        let mut job = Job::new_async(
            api.config.scheduler.trackers_fetch.clone(),
            move |_, scheduler| {
                let api = api.clone();
                Box::pin(async move {
                    if let Err(err) = Self::execute(api, scheduler).await {
                        error!("Failed to execute trackers fetch job: {err:?}");
                    }
                })
            },
        )?;

        job.set_job_type(SchedulerJob::TrackersFetch)?;

        Ok(job)
    }

    /// Executes a `TrackersFetch` job.
    async fn execute<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
        scheduler: JobScheduler,
    ) -> anyhow::Result<()>
    where
        ET::Error: EmailTransportError,
    {
        Self::fetch(api, scheduler).await?;

        Ok(())
    }

    async fn fetch<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
        scheduler: JobScheduler,
    ) -> anyhow::Result<()>
    where
        ET::Error: EmailTransportError,
    {
        // Fetch all trackers jobs that are pending processing.
        let trackers = api.trackers();
        let pending_trackers = trackers.get_pending_trackers();
        pin_mut!(pending_trackers);

        while let Some(tracker) = pending_trackers.next().await {
            let Some((tracker, job_id)) =
                Self::validate_tracker(&api, &scheduler, tracker?).await?
            else {
                continue;
            };

            // Try to create a new revision. If a revision is returned that means that tracker
            // detected changes.
            let fetch_start = Instant::now();
            let new_revision = match api
                .trackers()
                .create_tracker_data_revision(tracker.id)
                .await
            {
                Ok(new_revision) => new_revision,
                Err(err) => {
                    let execution_time = fetch_start.elapsed();
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
                        // Notify about the error and re-schedule the job.
                        let tracker_name = tracker.name.clone();
                        Self::try_notify(
                            &api,
                            tracker,
                            NotificationContentTemplate::TrackerChanges {
                                tracker_name,
                                content: Err(err
                                    .downcast::<RetrackError>()
                                    .map(|err| format!("{}", err))
                                    .unwrap_or_else(|_| "Unknown error".to_string())),
                            },
                        )
                        .await;
                        api.db.reset_scheduler_job_state(job_id, false).await?;
                    }

                    continue;
                }
            };

            let execution_time = fetch_start.elapsed();
            if let Some(revision) = new_revision {
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    metrics.job_execution_time = execution_time.as_nanos() as u64,
                    "Successfully checked tracker data — changes detected, and a new data revision has been created."
                );

                let tracker_name = tracker.name.clone();
                Self::try_notify(
                    &api,
                    tracker,
                    NotificationContentTemplate::TrackerChanges {
                        tracker_name,
                        content: Ok(revision.data),
                    },
                )
                .await;
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

        if tracker.config.revisions == 0 || tracker.config.job.is_none() {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %job_id,
                "Found a pending tracker that doesn't support tracking, the job will be removed."
            );

            scheduler.remove(&job_id).await?;
            api.trackers().update_tracker_job(tracker.id, None).await?;
            return Ok(None);
        }

        Ok(Some((tracker, job_id)))
    }

    async fn try_notify<DR: DnsResolver, ET: EmailTransport>(
        api: &Api<DR, ET>,
        tracker: Tracker,
        template: NotificationContentTemplate,
    ) where
        ET::Error: EmailTransportError,
    {
        let enable_notifications = tracker
            .config
            .job
            .as_ref()
            .and_then(|job_config| job_config.notifications)
            .unwrap_or_default();
        if !enable_notifications {
            return;
        }

        let notification_schedule_result = api
            .notifications()
            .schedule_notification(
                NotificationDestination::ServerLog,
                NotificationContent::Template(template),
                OffsetDateTime::now_utc(),
            )
            .await;
        if let Err(err) = notification_schedule_result {
            error!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Failed to schedule a notification for tracker: {err:?}."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TrackersFetchJob;
    use crate::{
        scheduler::{
            scheduler_job::SchedulerJob, scheduler_jobs::TrackersTriggerJob, SchedulerJobConfig,
            SchedulerJobRetryStrategy,
        },
        tests::{
            mock_api_with_config, mock_config, mock_get_scheduler_job, mock_schedule_in_sec,
            mock_schedule_in_secs, mock_scheduler, mock_scheduler_job, WebScraperContentRequest,
            WebScraperContentResponse, WebScraperErrorResponse,
        },
        trackers::{
            Tracker, TrackerConfig, TrackerCreateParams, TrackerDataRevision, TrackerTarget,
            WebPageTarget,
        },
    };
    use cron::Schedule;
    use futures::StreamExt;
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use std::{default::Default, ops::Add, sync::Arc, time::Duration};
    use time::OffsetDateTime;
    use url::Url;
    use uuid::{uuid, Uuid};

    #[sqlx::test]
    async fn can_create_job_with_correct_parameters(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_fetch = Schedule::try_from("1/5 * * * * *")?;

        let api = mock_api_with_config(pool, config).await?;

        let mut job = TrackersFetchJob::create(Arc::new(api)).await?;
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
        config.scheduler.trackers_fetch = Schedule::try_from("0 0 * * * *")?;

        let api = mock_api_with_config(pool, config).await?;

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        let job = TrackersFetchJob::try_resume(
            Arc::new(api),
            mock_scheduler_job(job_id, SchedulerJob::TrackersFetch, "0 0 * * * *"),
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
        config.scheduler.trackers_fetch = Schedule::try_from(mock_schedule_in_sec(2).as_str())?;

        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create tracker.
        let tracker_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), mock_schedule_in_sec(1)).await?)
            .await?;
        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParams {
                name: "tracker".to_string(),
                url: "https://localhost:1234/my/app?q=2".parse()?,
                target: Default::default(),
                config: TrackerConfig {
                    revisions: 0,
                    extractor: Default::default(),
                    headers: Default::default(),
                    job: Some(SchedulerJobConfig {
                        schedule: "0 0 * * * *".to_string(),
                        retry_strategy: None,
                        notifications: Some(true),
                    }),
                },
                tags: vec![],
            })
            .await?;
        api.trackers()
            .update_tracker_job(tracker.id, Some(tracker_job_id))
            .await?;

        // Schedule fetch job
        scheduler
            .add(TrackersFetchJob::create(api.clone()).await?)
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
    async fn can_fetch(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_fetch = Schedule::try_from(mock_schedule_in_sec(3).as_str())?;

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
            url: "https://localhost:1234/my/app?q=2".parse()?,
            target: TrackerTarget::WebPage(WebPageTarget {
                delay: Some(Duration::from_secs(2)),
                wait_for: Some("div".parse()?),
            }),
            config: TrackerConfig {
                revisions: 1,
                extractor: Some("return document.body.innerText;".to_string()),
                headers: Some(
                    [("cookie".to_string(), "my-cookie".to_string())]
                        .into_iter()
                        .collect(),
                ),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: None,
                    notifications: None,
                }),
            },
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

        // Schedule fetch job
        scheduler
            .add(TrackersFetchJob::create(api.clone()).await?)
            .await?;

        // Create a mock
        let content = WebScraperContentResponse {
            timestamp: OffsetDateTime::from_unix_timestamp(946720800)?,
            content: "some-content".to_string(),
        };

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
                        .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content);
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
                .map(|rev| (rev.created_at, rev.data))
                .collect::<Vec<_>>(),
            vec![(
                OffsetDateTime::from_unix_timestamp(946720800)?,
                content.content
            )]
        );

        // Check that the tracker job was marked as NOT stopped.
        let trigger_job = mock_get_scheduler_job(&api.db, trigger_job_id).await?;
        assert_eq!(
            trigger_job.map(|job| (job.id, job.stopped)),
            Some((trigger_job_id, Some(false)))
        );

        assert!(api
            .db
            .get_notification_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10
            )
            .collect::<Vec<_>>()
            .await
            .is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn schedules_notification_when_content_change(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_fetch = Schedule::try_from(mock_schedule_in_sec(3).as_str())?;

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
            url: "https://localhost:1234/my/app?q=2".parse()?,
            target: TrackerTarget::WebPage(WebPageTarget {
                delay: Some(Duration::from_secs(2)),
                wait_for: Some("div".parse()?),
            }),
            config: TrackerConfig {
                revisions: 2,
                extractor: Default::default(),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: None,
                    notifications: Some(true),
                }),
            },
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
                data: "some-content".to_string(),
            })
            .await?;

        // Schedule fetch job
        scheduler
            .add(TrackersFetchJob::create(api.clone()).await?)
            .await?;

        // Create a mock
        let content = WebScraperContentResponse {
            timestamp: OffsetDateTime::from_unix_timestamp(946720800)?,
            content: "other-content".to_string(),
        };
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content("some-content"),
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
            .get_notification_ids(
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

        let mut notification_ids = api
            .db
            .get_notification_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(notification_ids.len(), 1);

        let notification = api.db.get_notification(notification_ids.remove(0)?).await?;
        assert_debug_snapshot!(notification.map(|notification| (notification.destination, notification.content)), @r###"
        Some(
            (
                ServerLog,
                Template(
                    TrackerChanges {
                        tracker_name: "tracker-one",
                        content: Ok(
                            "other-content",
                        ),
                    },
                ),
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
    async fn schedules_notification_when_content_change_check_fails(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_fetch = Schedule::try_from(mock_schedule_in_sec(3).as_str())?;

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
            url: "https://localhost:1234/my/app?q=2".parse()?,
            target: TrackerTarget::WebPage(WebPageTarget {
                delay: Some(Duration::from_secs(2)),
                wait_for: Some("div".parse()?),
            }),
            config: TrackerConfig {
                revisions: 2,
                extractor: Default::default(),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: None,
                    notifications: Some(true),
                }),
            },
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
                data: "some-content".to_string(),
            })
            .await?;

        // Schedule fetch job
        scheduler
            .add(TrackersFetchJob::create(api.clone()).await?)
            .await?;

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content("some-content"),
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
            .get_notification_ids(
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

        let mut notification_ids = api
            .db
            .get_notification_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(notification_ids.len(), 1);

        let notification = api.db.get_notification(notification_ids.remove(0)?).await?;
        assert_debug_snapshot!(notification.map(|notification| (notification.destination, notification.content)), @r###"
        Some(
            (
                ServerLog,
                Template(
                    TrackerChanges {
                        tracker_name: "tracker-one",
                        content: Err(
                            "some client-error",
                        ),
                    },
                ),
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
        config.scheduler.trackers_fetch =
            Schedule::try_from(mock_schedule_in_secs(&[3, 6]).as_str())?;

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
            url: "https://localhost:1234/my/app?q=2".parse()?,
            target: TrackerTarget::WebPage(WebPageTarget {
                delay: Some(Duration::from_secs(2)),
                wait_for: Some("div".parse()?),
            }),
            config: TrackerConfig {
                revisions: 2,
                extractor: Default::default(),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(1),
                        max_attempts: 1,
                    }),
                    notifications: Some(true),
                }),
            },
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
                data: "some-content".to_string(),
            })
            .await?;

        // Schedule fetch job
        scheduler
            .add(TrackersFetchJob::create(api.clone()).await?)
            .await?;

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content("some-content"),
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

        let notification_ids = api
            .db
            .get_notification_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert!(notification_ids.is_empty());

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
            .get_notification_ids(
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

        let mut notification_ids = api
            .db
            .get_notification_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(notification_ids.len(), 1);

        let notification = api.db.get_notification(notification_ids.remove(0)?).await?;
        assert_debug_snapshot!(notification.map(|notification| (notification.destination, notification.content)), @r###"
        Some(
            (
                ServerLog,
                Template(
                    TrackerChanges {
                        tracker_name: "tracker-one",
                        content: Err(
                            "some client-error",
                        ),
                    },
                ),
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
        config.scheduler.trackers_fetch =
            Schedule::try_from(mock_schedule_in_secs(&[3, 6]).as_str())?;

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
            url: "https://localhost:1234/my/app?q=2".parse()?,
            target: TrackerTarget::WebPage(WebPageTarget {
                delay: Some(Duration::from_secs(2)),
                wait_for: Some("div".parse()?),
            }),
            config: TrackerConfig {
                revisions: 2,
                extractor: Default::default(),
                headers: Default::default(),
                job: Some(SchedulerJobConfig {
                    schedule: tracker_schedule,
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(1),
                        max_attempts: 1,
                    }),
                    notifications: Some(true),
                }),
            },
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
                data: "some-content".to_string(),
            })
            .await?;

        // Schedule fetch job
        scheduler
            .add(TrackersFetchJob::create(api.clone()).await?)
            .await?;

        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content("some-content"),
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

        let notification_ids = api
            .db
            .get_notification_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert!(notification_ids.is_empty());

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
        let content = WebScraperContentResponse {
            timestamp: OffsetDateTime::from_unix_timestamp(946720800)?,
            content: "other-content".to_string(),
        };
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content("some-content"),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content);
        });

        while api
            .db
            .get_notification_ids(
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

        let mut notification_ids = api
            .db
            .get_notification_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(notification_ids.len(), 1);

        let notification = api.db.get_notification(notification_ids.remove(0)?).await?;
        assert_debug_snapshot!(notification.map(|notification| (notification.destination, notification.content)), @r###"
        Some(
            (
                ServerLog,
                Template(
                    TrackerChanges {
                        tracker_name: "tracker-one",
                        content: Ok(
                            "other-content",
                        ),
                    },
                ),
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
