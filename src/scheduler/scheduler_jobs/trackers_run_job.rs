use crate::{
    api::Api,
    network::DnsResolver,
    scheduler::{
        CronExt, SchedulerJobMetadata, database_ext::RawSchedulerJobStoredData, job_ext::JobExt,
        scheduler_job::SchedulerJob,
    },
    tasks::{EmailContent, EmailTaskType, EmailTemplate, TaskType},
    trackers::{TrackerExecutionContext, TrackerExecutionRetry, TrackersApiExt},
};
use anyhow::Context;
use croner::Cron;
use retrack_types::trackers::Tracker;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{debug, error, warn};
use uuid::Uuid;

/// The job that is executed for every tracker with automatic tracking enabled and used to create
/// a new tracker data revision.
pub(crate) struct TrackersRunJob;
impl TrackersRunJob {
    /// Tries to resume existing `TrackersTrigger` job.
    pub async fn try_resume<DR: DnsResolver>(
        api: Arc<Api<DR>>,
        existing_job_data: RawSchedulerJobStoredData,
    ) -> anyhow::Result<Option<Job>> {
        // Check #1: The job should have a valid metadata. If not, remove it.
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

        // Check #2: The job should be associated with a tracker. If not, remove it.
        let trackers = api.trackers();
        let Some(tracker) = trackers.get_tracker_by_job_id(existing_job_data.id).await? else {
            warn!(
                job.id = %existing_job_data.id,
                "Job references tracker that doesn't exist, the job will be removed."
            );
            return Ok(None);
        };

        // Check #3: The tracker associated with the job should be enabled and set up for tracking.
        if !tracker.enabled || tracker.config.revisions == 0 {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %existing_job_data.id,
                "Tracker is disabled or doesn't support tracking, the job will be removed."
            );
            trackers.clear_tracker_job(tracker.id).await?;
            return Ok(None);
        }

        // Check #4: The tracker associated with the job should have schedule.
        let Some(job_config) = tracker.config.job else {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %existing_job_data.id,
                "Tracker no longer has a job config, the job will be removed."
            );
            trackers.clear_tracker_job(tracker.id).await?;
            return Ok(None);
        };

        // Check #5: If it's retry a job, but the tracker doesn't have a retry strategy, remove the job.
        if job_meta.retry_attempt > 0 && job_config.retry_strategy.is_none() {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %existing_job_data.id,
                "Tracker doesn't have a retry strategy, the retry job will be removed."
            );
            trackers.clear_tracker_job(tracker.id).await?;
            return Ok(None);
        }

        // Check #6: The job should have the same parameters as during job creation. Retry jobs
        // are persisted as `tokio_cron_scheduler` one-shots (`schedule = None`,
        // `job_type = OneShot`), reconstruct them via `create_retry` so the resumed `Job` is the
        // matching `NonCronJob` variant. Legacy retries persisted as date-pinned 6-field cron
        // strings (from before the one-shot migration) cannot be safely re-armed - clear the
        // tracker job and let `TrackersScheduleJob` re-schedule it with its real cron.
        let mut new_job = if job_meta.retry_attempt > 0 {
            if existing_job_data.schedule.is_some() {
                warn!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    job.id = %existing_job_data.id,
                    "Found legacy cron-style retry job, clearing tracker job to allow re-scheduling."
                );
                trackers.clear_tracker_job(tracker.id).await?;
                return Ok(None);
            }

            // The `Instant` here is a placeholder: `set_raw_job_data` below overwrites
            // `next_tick` with the persisted timestamp, and the scheduler reads that
            // persisted value (not the in-memory `Job`) when deciding when to fire.
            Self::create_retry(api.clone(), Instant::now() + Duration::from_secs(60))?
        } else {
            Self::create(api.clone(), &job_config.schedule)?
        };
        if !new_job.are_schedules_equal(&existing_job_data)? {
            debug!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %existing_job_data.id,
                "Tracker has changed schedule, the job will be removed."
            );
            trackers.clear_tracker_job(tracker.id).await?;
            return Ok(None);
        }

        // Resume the job via running a new job with the same content as before.
        new_job.set_raw_job_data(existing_job_data)?;

        debug!(
            tracker.id = %tracker.id,
            tracker.name = tracker.name,
            job.id = %new_job.guid(),
            "Successfully resumed tracker job."
        );
        Ok(Some(new_job))
    }

    /// Creates a new `TrackersTrigger` job.
    pub fn create<DR: DnsResolver>(
        api: Arc<Api<DR>>,
        schedule: impl AsRef<str>,
    ) -> anyhow::Result<Job> {
        // Now, create and schedule a new job.
        let mut job = Job::new_async(
            Cron::parse_pattern(schedule.as_ref())
                .with_context(|| format!("Cannot parse tracker's schedule: {}", schedule.as_ref()))?
                .pattern
                .to_string(),
            move |job_id, job_scheduler| {
                let api = api.clone();
                Box::pin(async move {
                    debug!(job.id = %job_id, "Running job.");
                    if let Err(err) = Self::execute(job_id, api, &job_scheduler).await {
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

    /// Creates a one-shot retry `TrackersRun` job that fires once at the given `instant`.
    ///
    /// Retries deliberately do not reuse the cron path: a date-pinned 6-field cron
    /// (e.g. `0 30 10 21 5 *`) parses in Quartz mode (day-of-month AND day-of-week,
    /// `dom_and_dow(true)`) and, once the persisted closure is dropped from
    /// `tokio_cron_scheduler`'s in-memory `SimpleJobCode` map, the next persisted tick is
    /// computed as the next matching date - typically a year out. A `OneShot` job has no
    /// schedule string at all (`schedule = None`, `job_type = OneShot`), after firing,
    /// `next_tick` is set to `None` and the row is reaped by the scheduler's
    /// `to_be_deleted` pass, so there is no future tick to drift.
    pub fn create_retry<DR: DnsResolver>(
        api: Arc<Api<DR>>,
        instant: Instant,
    ) -> anyhow::Result<Job> {
        let mut job = Job::new_one_shot_at_instant_async(instant, move |job_id, job_scheduler| {
            let api = api.clone();
            Box::pin(async move {
                debug!(job.id = %job_id, "Running retry job.");
                if let Err(err) = Self::execute(job_id, api, &job_scheduler).await {
                    error!(job.id = %job_id, "Retry job failed with unexpected error: {err:?}.");
                } else {
                    debug!(job.id = %job_id, "Finished running retry job.");
                }
            })
        })?;

        job.set_job_meta(&Self::create_job_meta())?;

        Ok(job)
    }

    async fn execute<DR: DnsResolver>(
        job_id: Uuid,
        api: Arc<Api<DR>>,
        job_scheduler: &JobScheduler,
    ) -> anyhow::Result<()> {
        // Check #1: The job should have valid metadata. If not, remove it.
        let run_start = Instant::now();
        let Some(mut job_meta) = api.scheduler().get_job_meta(job_id).await? else {
            error!(
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "The job doesn't have metadata and will be removed."
            );
            job_scheduler.remove(&job_id).await?;
            return Ok(());
        };

        // Check #2: The job should be associated with a tracker. If not, remove it.
        let trackers = api.trackers();
        let Some(tracker) = trackers.get_tracker_by_job_id(job_id).await? else {
            warn!(
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "Job references tracker that doesn't exist, the job will be removed."
            );
            job_scheduler.remove(&job_id).await?;
            return Ok(());
        };

        // Check #3: The tracker associated with the job should have a schedule.
        let Some(ref job_config) = tracker.config.job else {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "Tracker no longer has a job config, the job will be removed."
            );
            trackers.clear_tracker_job(tracker.id).await?;
            job_scheduler.remove(&job_id).await?;
            return Ok(());
        };

        // Check #4: If it's retry a job, but the tracker doesn't have a retry strategy, remove the job.
        if job_meta.retry_attempt > 0 && job_config.retry_strategy.is_none() {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "Tracker doesn't have a retry strategy, the retry job will be removed."
            );
            trackers.clear_tracker_job(tracker.id).await?;
            job_scheduler.remove(&job_id).await?;
            return Ok(());
        }

        // Check #5: The tracker associated with the job should be enabled and set up for tracking.
        // If not, remove the job and update the tracker to remove the job reference.
        if !tracker.enabled || tracker.config.revisions == 0 {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "Tracker is disabled or doesn't support automatic tracking, the job will be removed."
            );
            trackers.clear_tracker_job(tracker.id).await?;
            job_scheduler.remove(&job_id).await?;
            return Ok(());
        }

        // Check #6: The job shouldn't be already running. If not, skip it.
        if job_meta.is_running {
            debug!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                job.id = %job_id,
                metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                "The job is already running and will be skipped."
            );
            return Ok(());
        }

        // Now, mark the job as running, preserving the rest of the metadata.
        let scheduler = api.scheduler();
        scheduler
            .set_job_meta(job_id, job_meta.set_running())
            .await?;

        let context = TrackerExecutionContext {
            job_id: Some(job_id),
            retry: job_config.retry_strategy.map(|s| TrackerExecutionRetry {
                attempt: job_meta.retry_attempt,
                max_attempts: s.max_attempts() as u16,
            }),
        };

        // Try to create a new revision. If a revision is returned, that means that the tracker
        // detected changes.
        match trackers
            .create_tracker_data_revision(tracker.id, context)
            .await
        {
            Ok(new_revision) => {
                debug!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    job.id = %job_id,
                    metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                    metrics.tracker_data_size = new_revision.data.size(),
                    "Successfully checked tracker data."
                );

                // If it was a retry attempt, remove the job to let scheduler reschedule it using
                // the latest tracker schedule. Otherwise, just mark the job as not running.
                if job_meta.retry_attempt > 0 {
                    trackers.clear_tracker_job(tracker.id).await?;
                    job_scheduler.remove(&job_id).await?;
                } else {
                    scheduler
                        .set_job_meta(job_id, &Self::create_job_meta())
                        .await?;
                }
            }
            Err(err) => {
                error!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    job.id = %job_id,
                    metrics.job_execution_time = run_start.elapsed().as_nanos() as u64,
                    "Failed to create tracker data revision: {err:?}"
                );

                // Check if the tracker has a retry strategy.
                if let Some(retry_strategy) = job_config.retry_strategy {
                    // Check if there are still retries left.
                    if job_meta.retry_attempt >= retry_strategy.max_attempts() {
                        warn!(
                            tracker.id = %tracker.id,
                            tracker.name = tracker.name,
                            job.id = %job_id,
                            "Retry limit reached ('{}') for a scheduler job.",
                            job_meta.retry_attempt
                        );

                        // Remove the job and update the tracker to remove the job reference to
                        // allow the scheduling job to re-schedule tracker and report the error.
                        trackers.clear_tracker_job(tracker.id).await?;
                        job_scheduler.remove(&job_id).await?;
                        Self::report_error(&api, tracker, err).await;

                        return Ok(());
                    }

                    let retry_in = retry_strategy.interval(job_meta.retry_attempt);
                    warn!(
                        tracker.id = %tracker.id,
                        tracker.name = tracker.name,
                        job.id = %job_id,
                        metrics.job_retries = job_meta.retry_attempt + 1,
                        "Scheduled a one-shot retry to create tracker data revision in {}.",
                        humantime::format_duration(retry_in)
                    );

                    // Schedule the retry as a `tokio_cron_scheduler` one-shot. This avoids the
                    // year-pinned cron pattern that the previous implementation built from
                    // `now + retry_in`: a date-pinned 6-field cron parses in Quartz mode and,
                    // once the in-memory closure is lost for any reason, the scheduler advances
                    // `next_tick` to the next matching date - typically a year out. With a
                    // one-shot, `next_tick` is the absolute retry instant and is cleared to
                    // `None` after firing, so there is no future tick to drift.
                    let mut retry_job = Self::create_retry(api.clone(), Instant::now() + retry_in)?;
                    retry_job.set_job_meta(
                        Self::create_job_meta().set_retry_attempt(job_meta.retry_attempt + 1),
                    )?;

                    // Register the retry under a fresh job ID, repoint the tracker to it, and
                    // only then remove the currently running job. Reusing the original
                    // `job_id` for the retry races with `tokio-cron-scheduler`'s in-memory
                    // closure store (two independent listener tasks process the `add` and
                    // `remove` events on a shared `HashMap<Uuid, _>`); when the deletion
                    // is applied after the insertion, the retry's closure is wiped while
                    // its persisted metadata remains, and the next tick can't find a
                    // closure for the id. With distinct ids the two listeners touch
                    // different keys and cannot collide.
                    let new_job_id = job_scheduler.add(retry_job).await?;
                    trackers.set_tracker_job(tracker.id, new_job_id).await?;
                    job_scheduler.remove(&job_id).await?;
                } else {
                    // Resets job meta and report the error.
                    scheduler
                        .set_job_meta(job_id, &Self::create_job_meta())
                        .await?;
                    Self::report_error(&api, tracker, err).await;
                }
            }
        }

        Ok(())
    }

    async fn report_error<DR: DnsResolver>(api: &Api<DR>, tracker: Tracker, error: anyhow::Error) {
        let Some(ref smtp) = api.network.smtp else {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Failed to report failed tracker data check: SMTP configuration is missing."
            );
            return;
        };

        let Some(ref catch_all_recipient) = smtp.config.catch_all else {
            warn!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Failed to report failed tracker data check: catch-all recipient is missing."
            );
            return;
        };

        let email_task = TaskType::Email(EmailTaskType {
            to: vec![catch_all_recipient.recipient.clone()],
            content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                tracker_id: tracker.id,
                tracker_name: tracker.name.clone(),
                result: Err(error
                    .downcast::<crate::error::Error>()
                    .map(|err| format!("{err}"))
                    .unwrap_or_else(|_| "Unknown error".to_string())),
            }),
        });

        let task_tags = TrackersApiExt::<DR>::get_task_tags(&tracker, &email_task);
        let tasks_schedule_result = api
            .tasks()
            .schedule_task(email_task, task_tags, OffsetDateTime::now_utc())
            .await;
        if let Err(err) = tasks_schedule_result {
            error!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Failed to report failed tracker data check: {err:?}."
            );
        }
    }

    /// Creates a new `TrackersRun` job metadata.
    fn create_job_meta() -> SchedulerJobMetadata {
        SchedulerJobMetadata::new(SchedulerJob::TrackersRun)
    }
}

#[cfg(test)]
mod tests {
    use super::TrackersRunJob;
    use crate::{
        config::{SmtpConfig, TrackersConfig},
        scheduler::{SchedulerJob, SchedulerJobMetadata},
        tasks::{EmailContent, EmailTaskType, EmailTemplate, TaskType},
        tests::{
            MockSmtpServer, SmtpCatchAllConfig, TrackerCreateParamsBuilder, mock_api,
            mock_api_with_network, mock_get_scheduler_job, mock_network_with_smtp,
            mock_schedule_in_sec, mock_scheduler, mock_scheduler_job, mock_smtp, mock_smtp_config,
            mock_upsert_scheduler_job,
        },
    };
    use futures::StreamExt;
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use regex::Regex;
    use retrack_types::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{
            ApiTarget, TargetRequest, Tracker, TrackerConfig, TrackerDataValue, TrackerTarget,
            TrackerUpdateParams,
        },
    };
    use serde_json::json;
    use sqlx::PgPool;
    use std::{ops::Add, sync::Arc, time::Duration};
    use test_log::test;
    use time::OffsetDateTime;
    use uuid::{Uuid, uuid};

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_create_job_with_correct_parameters(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let mut job = TrackersRunJob::create(api.clone(), "0 0 * * * *")?;
        let job_data = job
            .job_data()
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job))?;

        assert_debug_snapshot!(job_data, @r###"
        (
            0,
            [
                2,
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
        )
        "###);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_resume_job(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;

        let mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 0 * * * *");
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let mut job = TrackersRunJob::try_resume(api.clone(), mock_job)
            .await?
            .unwrap();

        let job_data = job
            .job_data()
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job))?;
        assert_debug_snapshot!(job_data, @r###"
        (
            0,
            [
                2,
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
        )
        "###);

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        assert_eq!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .unwrap()
                .id,
            tracker.id
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_resume_retry_job(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker with retry config.
        let mut create_params = TrackerCreateParamsBuilder::new("tracker").build();
        create_params.config.job = Some(SchedulerJobConfig {
            schedule: "0 0 * * * *".to_string(),
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(60),
                max_attempts: 3,
            }),
        });
        let tracker = api.trackers().create_tracker(create_params).await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;

        // Persist a one-shot retry job (schedule = None, job_type = OneShot/2) at the
        // shape that `TrackersRunJob::create_retry` writes when scheduling a retry.
        let mut mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "");
        mock_job.schedule = None;
        mock_job.job_type = 2;
        mock_job.next_tick =
            Some((OffsetDateTime::now_utc() + Duration::from_secs(60)).unix_timestamp());
        mock_job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(2),
        )?);
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let mut job = TrackersRunJob::try_resume(api.clone(), mock_job)
            .await?
            .expect("one-shot retry job should be resumable");
        let job_data = job.job_data()?;
        // `job_type = 2` (OneShot) and `retry_attempt = 2` survive the resume round-trip.
        assert_eq!(job_data.job_type, 2);
        assert_eq!(
            SchedulerJobMetadata::try_from(job_data.extra.as_slice())?,
            *SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(2)
        );
        // The inner job is a `NonCronJob` (no cron schedule string).
        match job_data.job {
            Some(tokio_cron_scheduler::job::job_data_prost::job_stored_data::Job::NonCronJob(
                _,
            )) => {}
            other => panic!("expected NonCronJob, got {other:?}"),
        }

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        assert_eq!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .unwrap()
                .id,
            tracker.id
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_legacy_cron_style_retry_job(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker with retry config.
        let mut create_params = TrackerCreateParamsBuilder::new("tracker").build();
        create_params.config.job = Some(SchedulerJobConfig {
            schedule: "0 0 * * * *".to_string(),
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(60),
                max_attempts: 3,
            }),
        });
        let tracker = api.trackers().create_tracker(create_params).await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;

        // Mimic a retry job persisted by the pre-one-shot code path: a 6-field cron
        // (`<sec> <min> <hour> <dom> <month> *`) with a `retry_attempt > 0` metadata.
        // The new `try_resume` must recognise this as stale and unstick the tracker.
        let mut mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 10 10 10 5 *");
        mock_job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(2),
        )?);
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(
            job.is_none(),
            "legacy cron-style retry should be dropped from the scheduler"
        );

        // Tracker should now be unscheduled (job_id cleared) and visible to the scheduling job.
        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert_eq!(
            unscheduled_trackers,
            vec![Tracker {
                job_id: None,
                ..tracker
            }]
        );
        assert!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_job_if_does_not_have_meta(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");
        let mut mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 0 * * * *");
        mock_job.extra = None;
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_if_job_is_running(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker with retry config.
        let mut create_params = TrackerCreateParamsBuilder::new("tracker").build();
        create_params.config.job = Some(SchedulerJobConfig {
            schedule: "0 0 * * * *".to_string(),
            retry_strategy: None,
        });
        let tracker = api.trackers().create_tracker(create_params).await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;

        // Create associated tracker job.
        let mut mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 10 10 10 * *");
        mock_job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_running(),
        )?);
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_job_if_tracker_no_longer_exists(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");
        let mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 0 * * * *");
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_job_if_tracker_is_disabled(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("1 0 * * * *")
                    .disable()
                    .build(),
            )
            .await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;
        let mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "1 0 * * * *");
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_job_if_tracker_does_not_support_revisions(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("1 0 * * * *")
                    .with_config(TrackerConfig {
                        revisions: 0,
                        ..Default::default()
                    })
                    .build(),
            )
            .await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;
        let mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "1 0 * * * *");
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_job_if_tracker_no_longer_has_schedule(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("tracker").build())
            .await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;

        let mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 0 * * * *");
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(
            api.trackers()
                .get_tracker(tracker.id)
                .await?
                .unwrap()
                .job_id
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_job_if_schedule_changes(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("1 0 * * * *")
                    .build(),
            )
            .await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;
        let mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 0 * * * *");
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert_eq!(
            unscheduled_trackers,
            vec![Tracker {
                job_id: None,
                ..tracker
            }]
        );

        assert!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn removes_retry_job_if_tracker_no_longer_supports_retry(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker without retry config.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;
        api.trackers().set_tracker_job(tracker.id, job_id).await?;

        // Create associated tracker job.
        let mut mock_job = mock_scheduler_job(job_id, SchedulerJob::TrackersRun, "0 10 10 10 * *");
        mock_job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(2),
        )?);
        mock_upsert_scheduler_job(&api.db, &mock_job).await?;

        let job = TrackersRunJob::try_resume(api.clone(), mock_job).await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert_eq!(
            unscheduled_trackers,
            vec![Tracker {
                job_id: None,
                ..tracker
            }]
        );

        assert!(
            api.trackers()
                .get_tracker_by_job_id(job_id)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[test(sqlx::test)]
    async fn when_run_removes_job_if_does_not_reference_tracker(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api(pool).await?);

        // Create tracker job.
        let job_id = scheduler
            .add(TrackersRunJob::create(
                api.clone(),
                mock_schedule_in_sec(1),
            )?)
            .await?;
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        while mock_get_scheduler_job(&api.db, job_id).await?.is_some() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await?;

        Ok(())
    }

    #[test(sqlx::test)]
    async fn when_run_removes_job_if_tracker_does_not_support_tracking(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api(pool).await?);

        // Create tracker.
        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParamsBuilder::new("tracker").build())
            .await?;

        // Create tracker job.
        let job_id = scheduler
            .add(TrackersRunJob::create(
                api.clone(),
                mock_schedule_in_sec(1),
            )?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        while trackers.get_tracker_by_job_id(job_id).await?.is_some() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await?;

        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_none());

        Ok(())
    }

    #[test(sqlx::test)]
    async fn when_run_removes_retry_job_if_tracker_does_not_support_retries(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api(pool).await?);

        // Create tracker.
        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        // Create tracker job.
        let job_id = scheduler
            .add(TrackersRunJob::create(
                api.clone(),
                mock_schedule_in_sec(1),
            )?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;
        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());

        let mut job = mock_get_scheduler_job(&api.db, job_id).await?.unwrap();
        job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(2),
        )?);
        mock_upsert_scheduler_job(&api.db, &job).await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        while trackers.get_tracker_by_job_id(job_id).await?.is_some() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await?;

        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_none());

        Ok(())
    }

    #[test(sqlx::test)]
    async fn when_run_removes_job_if_tracker_does_not_support_revisions(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let api = Arc::new(mock_api(pool).await?);

        // Create tracker.
        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_config(TrackerConfig {
                        revisions: 0,
                        ..Default::default()
                    })
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        // Create tracker job.
        let job_id = scheduler
            .add(TrackersRunJob::create(
                api.clone(),
                mock_schedule_in_sec(1),
            )?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        while trackers.get_tracker_by_job_id(job_id).await?.is_some() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await?;

        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_none());

        Ok(())
    }

    #[test(sqlx::test)]
    async fn when_run_removes_job_if_tracker_is_disabled(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;
        let api = Arc::new(mock_api(pool).await?);

        // Create tracker.
        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .disable()
                    .build(),
            )
            .await?;

        // Create tracker job.
        let job_id = scheduler
            .add(TrackersRunJob::create(
                api.clone(),
                mock_schedule_in_sec(1),
            )?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        while trackers.get_tracker_by_job_id(job_id).await?.is_some() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await?;

        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_none());

        Ok(())
    }

    #[test(sqlx::test)]
    async fn can_run(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;
        let api = Arc::new(mock_api(pool).await?);
        let server = MockServer::start();

        // Create tracker.
        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-normal-job")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest::new(server.url("/api-normal-job").parse()?)],
                        configurator: None,
                        extractor: None,
                        params: None,
                    }))
                    .build(),
            )
            .await?;

        // Create tracker job.
        let job_schedule = mock_schedule_in_sec(1);
        let job_id = scheduler
            .add(TrackersRunJob::create(api.clone(), &job_schedule)?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Create server mock.
        let content = TrackerDataValue::new(json!("some-content"));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api-normal-job");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value())
                .delay(Duration::from_secs(2));
        });

        assert!(
            trackers
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        let mut was_running = false;
        loop {
            let job_meta = SchedulerJobMetadata::try_from(
                mock_get_scheduler_job(&api.db, job_id)
                    .await?
                    .unwrap()
                    .extra
                    .unwrap()
                    .as_ref(),
            )?;

            // Wait until job is marked as running, and then when it stops.
            if job_meta.is_running {
                was_running = true;
            } else if was_running {
                break;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;
        content_mock.assert();

        // Check that content was saved.
        assert_eq!(
            api.trackers()
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .into_iter()
                .map(|rev| rev.data)
                .collect::<Vec<_>>(),
            vec![content]
        );

        // Check that the job meta was reset
        let job = mock_get_scheduler_job(&api.db, job_id).await?.unwrap();
        assert_eq!(job.schedule, Some(job_schedule));
        assert_eq!(job.stopped, Some(false));

        let job_meta = SchedulerJobMetadata::try_from(job.extra.unwrap().as_ref())?;
        assert_eq!(
            job_meta,
            SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                is_running: false,
                retry_attempt: 0,
            }
        );

        Ok(())
    }

    #[test(sqlx::test)]
    async fn can_run_retry_job(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;
        let api = Arc::new(mock_api(pool).await?);
        let server = MockServer::start();

        // Create tracker with retry strategy.
        let mut create_params = TrackerCreateParamsBuilder::new("tracker-retry-job")
            .with_target(TrackerTarget::Api(ApiTarget {
                requests: vec![TargetRequest::new(server.url("/api-retry-job").parse()?)],
                configurator: None,
                extractor: None,
                params: None,
            }))
            .build();
        create_params.config.job = Some(SchedulerJobConfig {
            schedule: "0 0 * * * *".to_string(),
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(60),
                max_attempts: 3,
            }),
        });
        let trackers = api.trackers();
        let tracker = trackers.create_tracker(create_params).await?;

        // Create tracker job.
        let job_id = scheduler
            .add(TrackersRunJob::create(
                api.clone(),
                mock_schedule_in_sec(1),
            )?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;
        let mut job = mock_get_scheduler_job(&api.db, job_id).await?.unwrap();
        job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(2),
        )?);
        mock_upsert_scheduler_job(&api.db, &job).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Create server mock.
        let content = TrackerDataValue::new(json!("some-content"));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api-retry-job");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value())
                .delay(Duration::from_secs(2));
        });

        assert!(
            trackers
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        let mut was_running = false;
        loop {
            if let Some(job) = mock_get_scheduler_job(&api.db, job_id).await? {
                // Wait until job is marked as running, and then when it stops.
                let job_meta = SchedulerJobMetadata::try_from(job.extra.unwrap().as_ref())?;
                if job_meta.is_running {
                    was_running = true;
                }
            } else if was_running {
                break;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;
        content_mock.assert();

        // Check that content was saved.
        assert_eq!(
            api.trackers()
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .into_iter()
                .map(|rev| rev.data)
                .collect::<Vec<_>>(),
            vec![content]
        );

        // Check that the tracker retry job has been removed to be re-scheduled.
        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_none());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_none());

        Ok(())
    }

    #[test(sqlx::test)]
    async fn resets_job_and_report_error_if_job_fails(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let server = MockServer::start();
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let smtp_config = SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "dev@retrack.dev".to_string(),
                text_matcher: Regex::new(r"alpha")?,
            }),
            ..mock_smtp_config(smtp_server.host.to_string(), smtp_server.port)
        };
        let api = Arc::new(
            mock_api_with_network(pool, mock_network_with_smtp(mock_smtp(smtp_config))).await?,
        );

        // Create tracker.
        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-failed-job")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest::new(server.url("/api-failed-job").parse()?)],
                        configurator: None,
                        extractor: None,
                        params: None,
                    }))
                    .with_tags(vec!["tag1".to_string(), "tag2".to_string()])
                    .build(),
            )
            .await?;

        // Create the tracker job.
        let job_schedule = mock_schedule_in_sec(1);
        let job_id = scheduler
            .add(TrackersRunJob::create(api.clone(), &job_schedule)?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Create server mock.
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api-failed-job");
            then.status(400)
                .header("Content-Type", "application/json")
                .body("Uh oh (failed-job)!")
                .delay(Duration::from_secs(2));
        });

        assert!(
            trackers
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        let mut was_running = false;
        loop {
            let job_meta = SchedulerJobMetadata::try_from(
                mock_get_scheduler_job(&api.db, job_id)
                    .await?
                    .unwrap()
                    .extra
                    .unwrap()
                    .as_ref(),
            )?;

            // Wait until job is marked as running, and then when it stops.
            if job_meta.is_running {
                was_running = true;
            } else if was_running {
                break;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;
        content_mock.assert();

        // Check that content was NOT saved.
        assert!(
            trackers
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        // Check that the tracker job meta was reset.
        let job = mock_get_scheduler_job(&api.db, job_id).await?.unwrap();
        assert_eq!(job.schedule, Some(job_schedule));

        let job_meta = SchedulerJobMetadata::try_from(job.extra.unwrap().as_ref())?;
        assert_eq!(
            job_meta,
            SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                is_running: false,
                retry_attempt: 0,
            }
        );

        let mut tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let mut task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            task.task_type,
            TaskType::Email(EmailTaskType {
                to: vec!["dev@retrack.dev".to_string()],
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name,
                    result: Err(
                        "Failed to execute the API request (0): 400 Bad Request Uh oh (failed-job)!".to_string()
                    )
                })
            })
        );
        task.tags.sort();
        assert_eq!(
            task.tags,
            vec![
                "@retrack:task:type:email".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "tag1".to_string(),
                "tag2".to_string(),
            ]
        );

        Ok(())
    }

    #[test(sqlx::test)]
    async fn removes_job_and_report_error_if_last_retry_fails(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let server = MockServer::start();
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let smtp_config = SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "dev@retrack.dev".to_string(),
                text_matcher: Regex::new(r"alpha")?,
            }),
            ..mock_smtp_config(smtp_server.host.to_string(), smtp_server.port)
        };
        let api = Arc::new(
            mock_api_with_network(pool, mock_network_with_smtp(mock_smtp(smtp_config))).await?,
        );

        // Create tracker with retry strategy.
        let mut create_params = TrackerCreateParamsBuilder::new("tracker-failed-retry")
            .with_target(TrackerTarget::Api(ApiTarget {
                requests: vec![TargetRequest::new(server.url("/api-failed-retry").parse()?)],
                configurator: None,
                extractor: None,
                params: None,
            }))
            .build();
        create_params.config.job = Some(SchedulerJobConfig {
            schedule: "0 0 * * * *".to_string(),
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(60),
                max_attempts: 3,
            }),
        });
        let trackers = api.trackers();
        let tracker = trackers.create_tracker(create_params).await?;

        // Create a tracker job.
        let job_schedule = mock_schedule_in_sec(1);
        let job_id = scheduler
            .add(TrackersRunJob::create(api.clone(), &job_schedule)?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;
        let mut job = mock_get_scheduler_job(&api.db, job_id).await?.unwrap();
        job.extra = Some(Vec::try_from(
            &*SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(3),
        )?);
        mock_upsert_scheduler_job(&api.db, &job).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Create server mock.
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api-failed-retry");
            then.status(400)
                .header("Content-Type", "application/json")
                .body("Uh oh (failed retry)!")
                .delay(Duration::from_secs(2));
        });

        assert!(
            trackers
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;
        let mut was_running = false;
        loop {
            if let Some(job) = mock_get_scheduler_job(&api.db, job_id).await? {
                // Wait until job is marked as running, and then when it stops.
                let job_meta = SchedulerJobMetadata::try_from(job.extra.unwrap().as_ref())?;
                if job_meta.is_running {
                    was_running = true;
                }
            } else if was_running {
                break;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        scheduler.shutdown().await?;
        content_mock.assert();

        // Check that content was NOT saved.
        assert!(
            trackers
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        // Check that the tracker job meta was reset.
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_none());
        assert!(
            trackers
                .get_tracker(tracker.id)
                .await?
                .unwrap()
                .job_id
                .is_none()
        );

        let mut tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            task.task_type,
            TaskType::Email(EmailTaskType {
                to: vec!["dev@retrack.dev".to_string()],
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name,
                    result: Err(
                        "Failed to execute the API request (0): 400 Bad Request Uh oh (failed retry)!"
                            .to_string()
                    )
                })
            })
        );

        Ok(())
    }

    /// Regression test for the same-`job_id` race in `tokio-cron-scheduler`'s in-memory
    /// `SimpleJobCode` closure store. When a job fails and is rescheduled as a retry, the
    /// retry must be registered under a fresh `job_id` so that the closure insertion and
    /// removal listener tasks can never collide on the same HashMap key. If the retry
    /// reused the original `job_id`, the deletion listener could wipe the freshly-inserted
    /// closure while the persisted `scheduler_jobs` row remained, leading to the retry
    /// silently failing to fire and the row's `next_tick` snapping to the next match of
    /// the date-pinned retry cron — one year out.
    #[test(sqlx::test)]
    async fn retry_uses_fresh_job_id_after_failure(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;
        let server = MockServer::start();
        let api = Arc::new(mock_api(pool).await?);

        // Create a tracker with a retry strategy that points at a guaranteed-failing target.
        let mut create_params = TrackerCreateParamsBuilder::new("tracker-retry-id")
            .with_target(TrackerTarget::Api(ApiTarget {
                requests: vec![TargetRequest::new(server.url("/api-retry-id").parse()?)],
                configurator: None,
                extractor: None,
                params: None,
            }))
            .build();
        create_params.config.job = Some(SchedulerJobConfig {
            schedule: "0 0 * * * *".to_string(),
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(60),
                max_attempts: 3,
            }),
        });
        let trackers = api.trackers();
        let tracker = trackers.create_tracker(create_params).await?;

        // Register the initial job and link it to the tracker.
        let job_schedule = mock_schedule_in_sec(1);
        let original_job_id = scheduler
            .add(TrackersRunJob::create(api.clone(), &job_schedule)?)
            .await?;
        trackers
            .set_tracker_job(tracker.id, original_job_id)
            .await?;

        let fail_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api-retry-id");
            then.status(400)
                .header("Content-Type", "application/json")
                .body("Boom!")
                .delay(Duration::from_secs(1));
        });

        // Start the scheduler and wait for the tracker's `job_id` to be repointed at the
        // retry's fresh id (or give up after ~20s).
        scheduler.start().await?;
        let mut new_job_id: Option<Uuid> = None;
        for _ in 0..200 {
            if let Some(t) = trackers.get_tracker(tracker.id).await?
                && let Some(id) = t.job_id
                && id != original_job_id
            {
                new_job_id = Some(id);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await?;
        fail_mock.assert();

        // The retry must have been registered under a different job id (the regression
        // guard) and the original `scheduler_jobs` row must be gone.
        let new_job_id = new_job_id.expect(
            "retry should have repointed tracker.job_id to a fresh id (same-id race regression)",
        );
        assert_ne!(new_job_id, original_job_id);
        assert!(
            mock_get_scheduler_job(&api.db, original_job_id)
                .await?
                .is_none(),
            "original scheduler_jobs row should be removed once the retry is registered"
        );

        // The new `scheduler_jobs` row must be a `tokio_cron_scheduler` one-shot:
        // `schedule = None`, `job_type = OneShot` (= 2), `next_tick` in the near future,
        // and `retry_attempt = 1` in its metadata.
        let new_job = mock_get_scheduler_job(&api.db, new_job_id)
            .await?
            .expect("retry scheduler_jobs row must exist");
        assert!(
            new_job.schedule.is_none(),
            "one-shot retry must not carry a cron schedule string: {:?}",
            new_job.schedule
        );
        assert_eq!(
            new_job.job_type, 2,
            "one-shot retry must persist `job_type = OneShot` (2)"
        );
        let next_tick = new_job
            .next_tick
            .expect("one-shot retry must have a concrete next_tick");
        let now = OffsetDateTime::now_utc().unix_timestamp();
        assert!(
            next_tick >= now - 5 && next_tick <= now + 120,
            "retry next_tick should sit near `now + retry_interval` (60s): next_tick={next_tick}, now={now}"
        );
        let meta = SchedulerJobMetadata::try_from(new_job.extra.as_deref().unwrap())?;
        assert_eq!(meta.job_type, SchedulerJob::TrackersRun);
        assert_eq!(meta.retry_attempt, 1);
        assert!(!meta.is_running);

        Ok(())
    }

    #[test(sqlx::test)]
    async fn can_schedule_retries_if_request_fails(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;

        let server = MockServer::start();
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let smtp_config = SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "dev@retrack.dev".to_string(),
                text_matcher: Regex::new(r"alpha")?,
            }),
            ..mock_smtp_config(smtp_server.host.to_string(), smtp_server.port)
        };
        let mut api =
            mock_api_with_network(pool, mock_network_with_smtp(mock_smtp(smtp_config))).await?;
        api.config.trackers = TrackersConfig {
            restrict_to_public_urls: false,
            min_retry_interval: Duration::from_secs(2),
            ..Default::default()
        };
        let api = Arc::new(api);

        // Create tracker with retry strategy.
        let api_url = server.url("/api-retry");
        let mut create_params = TrackerCreateParamsBuilder::new("tracker-with-retry")
            .with_target(TrackerTarget::Api(ApiTarget {
                requests: vec![TargetRequest::new(api_url.parse()?)],
                configurator: Some(format!("((context) => ({{ requests: [{{ url: '{api_url}', headers: {{ 'x-custom-header': context.tags[0] }} }}] }}))(context);")),
                extractor: None,
                params: None,
            }))
            .with_tags(vec!["attempt-1".to_string()])
            .build();
        create_params.config.job = Some(SchedulerJobConfig {
            schedule: "0 0 * * * *".to_string(),
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(3),
                max_attempts: 2,
            }),
        });
        let trackers = api.trackers();
        let tracker = trackers.create_tracker(create_params).await?;

        // Create tracker job.
        let job_schedule = mock_schedule_in_sec(2);
        let job_id = scheduler
            .add(TrackersRunJob::create(api.clone(), &job_schedule)?)
            .await?;
        trackers.set_tracker_job(tracker.id, job_id).await?;

        assert!(trackers.get_tracker_by_job_id(job_id).await?.is_some());
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_some());

        // Create server mock (original request and first retry fail, second retry succeeds).
        let attempt_one_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .header("x-custom-header", "attempt-1")
                .path("/api-retry");
            then.status(400)
                .header("Content-Type", "application/json")
                .body("Uh oh (first attempt)!")
                .delay(Duration::from_secs(3));
        });

        let attempt_two_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .header("x-custom-header", "attempt-2")
                .path("/api-retry");
            then.status(400)
                .header("Content-Type", "application/json")
                .body("Uh oh (second attempt)!")
                .delay(Duration::from_secs(3));
        });

        let content = TrackerDataValue::new(json!("Hooray!!!"));
        let attempt_three_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .header("x-custom-header", "attempt-3")
                .path("/api-retry");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value())
                .delay(Duration::from_secs(3));
        });

        assert!(
            trackers
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        // Start scheduler and wait for a few seconds, then stop it. Each retry attempt is
        // registered under a fresh `job_id` (see the failure branch of
        // `TrackersRunJob::execute`), so the loop follows the tracker's current `job_id`
        // rather than the original one, and asserts that the id changes between attempts.
        scheduler.start().await?;
        let mut should_increase_counter = true;
        let mut attempts_counter = 1;
        let mut observed_job_ids: Vec<Uuid> = vec![job_id];
        loop {
            let current_job_id = trackers
                .get_tracker(tracker.id)
                .await?
                .and_then(|t| t.job_id);

            if let Some(current_job_id) = current_job_id {
                if observed_job_ids.last() != Some(&current_job_id) {
                    observed_job_ids.push(current_job_id);
                }

                if let Some(job) = mock_get_scheduler_job(&api.db, current_job_id).await? {
                    // Every time the job is marked as running, update tracker tags with the
                    // attempt number, so that configurator script can change HTTP request
                    // header and we can use separate mocks for every attempt.
                    let job_meta = SchedulerJobMetadata::try_from(job.extra.unwrap().as_ref())?;
                    if job_meta.is_running && should_increase_counter {
                        attempts_counter += 1;
                        let update_params = TrackerUpdateParams {
                            tags: Some(vec![format!("attempt-{attempts_counter}")]),
                            ..Default::default()
                        };
                        trackers.update_tracker(tracker.id, update_params).await?;

                        should_increase_counter = false;
                    } else if !job_meta.is_running {
                        should_increase_counter = true;
                    }
                }
            } else if attempts_counter >= 4 {
                break;
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Each failure should have re-registered the retry under a fresh job_id (the
        // regression guard for the same-id race in `tokio-cron-scheduler`'s closure store).
        assert!(
            observed_job_ids.len() >= 3,
            "expected at least 3 distinct job ids across original + 2 retries, got {observed_job_ids:?}"
        );
        for window in observed_job_ids.windows(2) {
            assert_ne!(window[0], window[1], "retry must use a fresh job id");
        }

        scheduler.shutdown().await?;

        attempt_one_mock.assert();
        attempt_two_mock.assert();
        attempt_three_mock.assert();

        // Check that we eventually got data.
        assert_eq!(
            api.trackers()
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .into_iter()
                .map(|rev| rev.data)
                .collect::<Vec<_>>(),
            vec![content]
        );

        // Check that the tracker job meta was reset, so that it can be re-scheduled after retries.
        assert!(mock_get_scheduler_job(&api.db, job_id).await?.is_none());
        assert!(
            trackers
                .get_tracker(tracker.id)
                .await?
                .unwrap()
                .job_id
                .is_none()
        );

        // Check that error wasn't reported.
        let tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        Ok(())
    }
}
