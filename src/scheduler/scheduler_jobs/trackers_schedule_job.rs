use crate::{
    api::Api,
    network::DnsResolver,
    scheduler::{
        CronExt, SchedulerJobMetadata, database_ext::RawSchedulerJobStoredData, job_ext::JobExt,
        scheduler_job::SchedulerJob, scheduler_jobs::TrackersRunJob,
    },
};
use anyhow::Context;
use croner::Cron;
use retrack_types::trackers::Tracker;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use time::OffsetDateTime;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{debug, error, info};

/// Minimum interval between execution log cleanup runs (1 hour).
const CLEANUP_INTERVAL_SECS: i64 = 3600;

/// Tracks the last time execution log cleanup was performed (Unix timestamp in seconds).
static LAST_CLEANUP_TIMESTAMP: AtomicI64 = AtomicI64::new(0);

/// The job executes every minute by default to check if there are any trackers to schedule jobs for.
pub(crate) struct TrackersScheduleJob;
impl TrackersScheduleJob {
    /// Tries to resume existing `TrackersSchedule` job.
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

    /// Creates a new `TrackersSchedule` job.
    pub fn create<DR: DnsResolver>(api: Arc<Api<DR>>) -> anyhow::Result<Job> {
        let mut job = Job::new_async(
            Cron::parse_pattern(&api.config.scheduler.trackers_schedule)
                .with_context(|| {
                    format!(
                        "Cannot parse `trackers_schedule` schedule: {}",
                        api.config.scheduler.tasks_run
                    )
                })?
                .pattern
                .to_string(),
            move |job_id, scheduler| {
                let api = api.clone();
                Box::pin(async move {
                    debug!(job.id = %job_id, "Running job.");
                    if let Err(err) = Self::execute(api, scheduler).await {
                        error!(job.id = %job_id, "Job failed with unexpected error: {err:?}.");
                    } else {
                        debug!(job.id = %job_id, "Finished running job.");
                    }
                })
            },
        )?;

        job.set_job_meta(&SchedulerJobMetadata::new(SchedulerJob::TrackersSchedule))?;

        Ok(job)
    }

    /// Executes a `TrackersSchedule` job.
    async fn execute<DR: DnsResolver>(
        api: Arc<Api<DR>>,
        scheduler: JobScheduler,
    ) -> anyhow::Result<()> {
        let trackers = api.trackers();
        Self::schedule_trackers(
            api.clone(),
            &scheduler,
            trackers.get_trackers_to_schedule().await?,
        )
        .await?;

        Self::maybe_cleanup_execution_logs(&api).await;

        Ok(())
    }

    /// Runs execution log cleanup if enough time has elapsed since the last cleanup.
    async fn maybe_cleanup_execution_logs<DR: DnsResolver>(api: &Api<DR>) {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let last = LAST_CLEANUP_TIMESTAMP.load(Ordering::Relaxed);
        if now - last < CLEANUP_INTERVAL_SECS {
            return;
        }

        let retention = api.config.trackers.execution_log_retention;
        let cutoff =
            OffsetDateTime::now_utc() - time::Duration::seconds(retention.as_secs() as i64);

        match api.trackers().cleanup_tracker_execution_logs(cutoff).await {
            Ok(deleted) => {
                LAST_CLEANUP_TIMESTAMP.store(now, Ordering::Relaxed);
                if deleted > 0 {
                    info!("Cleaned up {deleted} expired tracker execution log entries.");
                }
            }
            Err(err) => {
                error!("Failed to clean up tracker execution logs: {err:?}");
            }
        }
    }

    async fn schedule_trackers<DR: DnsResolver>(
        api: Arc<Api<DR>>,
        scheduler: &JobScheduler,
        unscheduled_trackers: Vec<Tracker>,
    ) -> anyhow::Result<()> {
        if !unscheduled_trackers.is_empty() {
            debug!("Found {} unscheduled trackers.", unscheduled_trackers.len());
        }

        for tracker in unscheduled_trackers {
            let schedule = if let Some(job_config) = tracker.config.job {
                job_config.schedule
            } else {
                error!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    "Found an unscheduled tracker that doesn't have tracking schedule, skipping…"
                );
                continue;
            };

            // Now, create and schedule a new job.
            let job_id = scheduler
                .add(TrackersRunJob::create(api.clone(), schedule)?)
                .await?;
            api.trackers().set_tracker_job(tracker.id, job_id).await?;
            debug!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %job_id,
                "Successfully scheduled tracker."
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{LAST_CLEANUP_TIMESTAMP, TrackersScheduleJob};
    use crate::{
        scheduler::{SchedulerJobMetadata, scheduler_job::SchedulerJob},
        tests::{
            TrackerCreateParamsBuilder, mock_api_with_config, mock_config, mock_scheduler,
            mock_scheduler_job,
        },
    };
    use futures::StreamExt;
    use insta::assert_debug_snapshot;
    use retrack_types::trackers::{TrackerConfig, TrackerExecutionLog, TrackerExecutionLogStatus};
    use sqlx::PgPool;
    use std::{
        sync::{Arc, atomic::Ordering},
        time::Duration,
    };
    use time::OffsetDateTime;
    use uuid::{Uuid, uuid};

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_create_job_with_correct_parameters(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_schedule = "1/5 * * * * *".to_string();

        let api = mock_api_with_config(pool, config).await?;

        let mut job = TrackersScheduleJob::create(Arc::new(api))?;
        let job_data = job
            .job_data()
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job))?;
        assert_debug_snapshot!(job_data, @r###"
        (
            0,
            [
                1,
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

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_resume_job(pool: PgPool) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        config.scheduler.trackers_schedule = "0 0 * * * *".to_string();

        let api = mock_api_with_config(pool, config).await?;

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        let job = TrackersScheduleJob::try_resume(
            Arc::new(api),
            mock_scheduler_job(job_id, SchedulerJob::TrackersSchedule, "0 0 * * * *"),
        )?;
        let job_data = job
            .and_then(|mut job| job.job_data().ok())
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job));
        assert_debug_snapshot!(job_data, @r###"
        Some(
            (
                0,
                [
                    1,
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

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn resets_job_if_schedule_changed(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api_with_config(pool, mock_config()?).await?;

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        let job = TrackersScheduleJob::try_resume(
            Arc::new(api),
            mock_scheduler_job(job_id, SchedulerJob::TrackersSchedule, "0 0 * * * *"),
        )?;
        assert!(job.is_none());

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_schedule_trackers_jobs(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let mut config = mock_config()?;
        config.scheduler.trackers_schedule = "1/1 * * * * *".to_string();
        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create trackers and tracker jobs.
        let trackers = api.trackers();
        let tracker_one = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-one")
                    .with_schedule("1 2 3 4 5 4")
                    .build(),
            )
            .await?;
        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-two")
                    .with_schedule("1 2 3 4 5 5")
                    .build(),
            )
            .await?;
        let tracker_three = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-three")
                    .with_schedule("1 2 3 4 5 6")
                    .build(),
            )
            .await?;

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert_eq!(unscheduled_trackers.len(), 3);
        assert_eq!(unscheduled_trackers[0].id, tracker_one.id);
        assert_eq!(unscheduled_trackers[1].id, tracker_two.id);
        assert_eq!(unscheduled_trackers[2].id, tracker_three.id);

        scheduler
            .add(TrackersScheduleJob::create(api.clone())?)
            .await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        while !api.trackers().get_trackers_to_schedule().await?.is_empty() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await?;

        // All pending jobs should be scheduled now.
        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        let jobs = api
            .db
            .get_scheduler_jobs(10)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()?;
        assert_eq!(jobs.len(), 4);

        let trackers = api.trackers();
        let tracker_jobs = jobs
            .iter()
            .filter_map(|job_data| {
                let job_meta =
                    SchedulerJobMetadata::try_from(job_data.extra.as_deref().unwrap()).unwrap();
                if matches!(job_meta.job_type, SchedulerJob::TrackersRun) {
                    Some(job_data.id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(tracker_jobs.len(), 3);
        for job_id in tracker_jobs {
            let scheduled_tracker = trackers.get_tracker_by_job_id(job_id).await?;
            assert!(scheduled_tracker.is_some());
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn does_not_schedule_trackers_without_schedule(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let mut config = mock_config()?;
        config.scheduler.trackers_schedule = "1/1 * * * * *".to_string();

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-one").build())
            .await?;

        assert!(api.trackers().get_trackers_to_schedule().await?.is_empty());

        let schedule_job_id = scheduler
            .add(TrackersScheduleJob::create(api.clone())?)
            .await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        tokio::time::sleep(Duration::from_millis(2000)).await;
        scheduler.shutdown().await?;

        // Tracker has not been assigned job ID.
        assert!(
            api.trackers()
                .get_tracker(tracker.id)
                .await?
                .unwrap()
                .job_id
                .is_none()
        );

        let mut jobs = api.db.get_scheduler_jobs(10).collect::<Vec<_>>().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs.remove(0)?.id, schedule_job_id);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn does_not_schedule_trackers_if_revisions_is_zero(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let mut config = mock_config()?;
        config.scheduler.trackers_schedule = "1/1 * * * * *".to_string();

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-one")
                    .with_config(TrackerConfig {
                        revisions: 0,
                        ..Default::default()
                    })
                    .with_schedule("1 2 3 4 5 6")
                    .build(),
            )
            .await?;

        assert!(api.trackers().get_trackers_to_schedule().await?.is_empty());

        let schedule_job_id = scheduler
            .add(TrackersScheduleJob::create(api.clone())?)
            .await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        tokio::time::sleep(Duration::from_millis(2000)).await;
        scheduler.shutdown().await?;

        // Tracker has not been assigned job ID.
        assert!(
            api.trackers()
                .get_tracker(tracker.id)
                .await?
                .unwrap()
                .job_id
                .is_none()
        );

        let mut jobs = api.db.get_scheduler_jobs(10).collect::<Vec<_>>().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs.remove(0)?.id, schedule_job_id);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn does_not_schedule_trackers_if_disabled(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let mut config = mock_config()?;
        config.scheduler.trackers_schedule = "1/1 * * * * *".to_string();

        let api = Arc::new(mock_api_with_config(pool, config).await?);

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-one")
                    .with_schedule("1 2 3 4 5 6")
                    .disable()
                    .build(),
            )
            .await?;

        assert!(api.trackers().get_trackers_to_schedule().await?.is_empty());

        let schedule_job_id = scheduler
            .add(TrackersScheduleJob::create(api.clone())?)
            .await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        tokio::time::sleep(Duration::from_millis(2000)).await;
        scheduler.shutdown().await?;

        // Tracker has not been assigned job ID.
        assert!(
            api.trackers()
                .get_tracker(tracker.id)
                .await?
                .unwrap()
                .job_id
                .is_none()
        );

        let mut jobs = api.db.get_scheduler_jobs(10).collect::<Vec<_>>().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs.remove(0)?.id, schedule_job_id);

        Ok(())
    }

    fn mock_execution_log(
        tracker_id: Uuid,
        started_at: OffsetDateTime,
        status: TrackerExecutionLogStatus,
    ) -> TrackerExecutionLog {
        TrackerExecutionLog {
            id: Uuid::now_v7(),
            tracker_id,
            job_id: None,
            started_at,
            finished_at: started_at + time::Duration::seconds(1),
            status,
            error: if status == TrackerExecutionLogStatus::Failure {
                Some("test error".to_string())
            } else {
                None
            },
            is_manual: true,
            retry_attempt: None,
            max_retry_attempts: None,
            revision_size: Some(100),
            has_changes: None,
            duration_ms: 1000,
            phases: None,
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cleanup_deletes_expired_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api_with_config(pool, mock_config()?).await?;

        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("tracker").build())
            .await?;

        // Insert a log far beyond the 90-day default retention.
        let old_time = OffsetDateTime::from_unix_timestamp(946720800)?;
        api.trackers()
            .log_tracker_execution(&mock_execution_log(
                tracker.id,
                old_time,
                TrackerExecutionLogStatus::Success,
            ))
            .await;

        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert_eq!(logs.len(), 1);

        LAST_CLEANUP_TIMESTAMP.store(0, Ordering::Relaxed);
        TrackersScheduleJob::maybe_cleanup_execution_logs(&api).await;

        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert!(logs.is_empty());

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cleanup_preserves_recent_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api_with_config(pool, mock_config()?).await?;

        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("tracker").build())
            .await?;

        // Insert a recent log (within retention window).
        api.trackers()
            .log_tracker_execution(&mock_execution_log(
                tracker.id,
                OffsetDateTime::now_utc(),
                TrackerExecutionLogStatus::Success,
            ))
            .await;

        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert_eq!(logs.len(), 1);

        LAST_CLEANUP_TIMESTAMP.store(0, Ordering::Relaxed);
        TrackersScheduleJob::maybe_cleanup_execution_logs(&api).await;

        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert_eq!(logs.len(), 1);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cleanup_skips_when_interval_not_elapsed(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api_with_config(pool, mock_config()?).await?;

        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("tracker").build())
            .await?;

        // Insert a log far beyond retention.
        let old_time = OffsetDateTime::from_unix_timestamp(946720800)?;
        api.trackers()
            .log_tracker_execution(&mock_execution_log(
                tracker.id,
                old_time,
                TrackerExecutionLogStatus::Failure,
            ))
            .await;

        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert_eq!(logs.len(), 1);

        // Pretend cleanup just ran by setting the timestamp to now.
        LAST_CLEANUP_TIMESTAMP.store(
            OffsetDateTime::now_utc().unix_timestamp(),
            Ordering::Relaxed,
        );
        TrackersScheduleJob::maybe_cleanup_execution_logs(&api).await;

        // The old log should still be present because cleanup was skipped.
        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert_eq!(logs.len(), 1);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cleanup_deletes_only_expired_logs(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api_with_config(pool, mock_config()?).await?;

        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("tracker").build())
            .await?;

        // Insert an expired log and a recent log.
        let old_time = OffsetDateTime::from_unix_timestamp(946720800)?;
        api.trackers()
            .log_tracker_execution(&mock_execution_log(
                tracker.id,
                old_time,
                TrackerExecutionLogStatus::Failure,
            ))
            .await;
        api.trackers()
            .log_tracker_execution(&mock_execution_log(
                tracker.id,
                OffsetDateTime::now_utc(),
                TrackerExecutionLogStatus::Success,
            ))
            .await;

        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert_eq!(logs.len(), 2);

        LAST_CLEANUP_TIMESTAMP.store(0, Ordering::Relaxed);
        TrackersScheduleJob::maybe_cleanup_execution_logs(&api).await;

        let logs = api
            .trackers()
            .get_tracker_execution_logs(tracker.id, Default::default())
            .await?;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, TrackerExecutionLogStatus::Success);

        Ok(())
    }
}
