use crate::{
    api::Api,
    network::{DnsResolver, EmailTransport},
    scheduler::{
        database_ext::RawSchedulerJobStoredData, job_ext::JobExt, scheduler_job::SchedulerJob,
    },
};
use std::sync::Arc;
use tokio_cron_scheduler::Job;
use tracing::{debug, error, warn};

/// The job that is executed for every tracker with automatic tracking enabled. The job doesn't do
/// anything except logging, and updating its internal state. This job is supposed to be as
/// lightweight as possible since we might have thousands of them. There are dedicated schedule and
/// fetch jobs that batch all trackers that need to be scheduled and checked for changes respectively.
pub(crate) struct TrackersTriggerJob;
impl TrackersTriggerJob {
    /// Tries to resume existing `TrackersTrigger` job.
    pub async fn try_resume<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
        existing_job_data: RawSchedulerJobStoredData,
    ) -> anyhow::Result<Option<Job>> {
        // First, check if the tracker job exists.
        let trackers = api.trackers();
        let Some((tracker_id, tracker_name, tracker_config)) = trackers
            .get_tracker_by_job_id(existing_job_data.id)
            .await?
            .map(|tracker| (tracker.id, tracker.name, tracker.config))
        else {
            warn!(
                job.id = %existing_job_data.id,
                "Tracker job reference doesn't exist, the job will be removed."
            );
            return Ok(None);
        };

        // Then, check if the tracker can support revisions.
        if tracker_config.revisions == 0 {
            warn!(
                tracker.id = %tracker_id,
                job.id = %existing_job_data.id,
                "Tracker cannot store revisions, the job will be removed."
            );
            trackers.update_tracker_job(tracker_id, None).await?;
            return Ok(None);
        };

        // Then, check if the tracker still has a schedule.
        let Some(job_config) = tracker_config.job else {
            warn!(
                tracker.id = %tracker_id,
                job.id = %existing_job_data.id,
                "Tracker no longer has a job config, the job will be removed."
            );
            trackers.update_tracker_job(tracker_id, None).await?;
            return Ok(None);
        };

        // If we changed the job parameters, we need to remove the old job and create a new one.
        let mut new_job = Self::create(api.clone(), job_config.schedule).await?;
        Ok(if new_job.are_schedules_equal(&existing_job_data)? {
            debug!(
                tracker.id = %tracker_id,
                tracker.name = tracker_name,
                job.id = %existing_job_data.id,
                "Successfully resumed tracker job."
            );
            new_job.set_raw_job_data(existing_job_data)?;
            Some(new_job)
        } else {
            debug!(
                tracker.id = %tracker_id,
                tracker.name = tracker_name,
                job.id = %existing_job_data.id,
                "Tracker job will be removed and a new job will re-scheduled."
            );
            trackers.update_tracker_job(tracker_id, None).await?;
            None
        })
    }

    /// Creates a new `TrackersTrigger` job.
    pub async fn create<DR: DnsResolver, ET: EmailTransport>(
        api: Arc<Api<DR, ET>>,
        schedule: impl AsRef<str>,
    ) -> anyhow::Result<Job> {
        // Now, create and schedule new job.
        let mut job = Job::new_async(schedule.as_ref(), move |uuid, _| {
            let db = api.db.clone();
            Box::pin(async move {
                // Mark job as stopped to indicate that it needs processing. Schedule job only picks
                // up stopped jobs, processes them, and then un-stops. Stopped flag is basically
                // serving as a pending processing flag. Eventually we might need to add a separate
                // table for pending jobs.
                if let Err(err) = db.reset_scheduler_job_state(uuid, true).await {
                    error!(
                        job.id = %uuid,
                        "Error marking tracker trigger job as pending: {err:?}"
                    );
                } else {
                    debug!(job.id = %uuid, "Successfully run the job.");
                }
            })
        })?;

        job.set_job_type(SchedulerJob::TrackersTrigger)?;

        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::TrackersTriggerJob;
    use crate::{
        scheduler::{scheduler_job::SchedulerJob, SchedulerJobConfig},
        tests::{mock_api, mock_get_scheduler_job, mock_scheduler, mock_scheduler_job},
        trackers::{Tracker, TrackerConfig, TrackerCreateParams},
    };
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use std::sync::Arc;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_create_job_with_correct_parameters(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let mut job = TrackersTriggerJob::create(api.clone(), "0 0 * * * *").await?;
        let job_data = job.job_data().map(|job_data| {
            (
                job_data.job_type,
                job_data.extra,
                job_data.job,
                job_data.stopped,
            )
        })?;

        assert_debug_snapshot!(job_data, @r###"
        (
            0,
            [
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
            false,
        )
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn can_resume_job(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParams {
                name: "tracker".to_string(),
                url: "https://localhost:1234/my/app?q=2".parse()?,
                target: Default::default(),
                config: TrackerConfig {
                    revisions: 4,
                    extractor: Default::default(),
                    headers: Default::default(),
                    job: Some(SchedulerJobConfig {
                        schedule: "0 0 * * * *".to_string(),
                        retry_strategy: None,
                        notifications: Some(true),
                    }),
                },
            })
            .await?;
        api.trackers()
            .update_tracker_job(tracker.id, Some(job_id))
            .await?;

        let mut job = TrackersTriggerJob::try_resume(
            api.clone(),
            mock_scheduler_job(job_id, SchedulerJob::TrackersTrigger, "0 0 * * * *"),
        )
        .await?
        .unwrap();

        let job_data = job
            .job_data()
            .map(|job_data| (job_data.job_type, job_data.extra, job_data.job))?;
        assert_debug_snapshot!(job_data, @r###"
        (
            3,
            [
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

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
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

    #[sqlx::test]
    async fn resets_job_if_schedule_changes(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParams {
                name: "tracker".to_string(),
                url: "https://localhost:1234/my/app?q=2".parse()?,
                target: Default::default(),
                config: TrackerConfig {
                    revisions: 4,
                    extractor: Default::default(),
                    headers: Default::default(),
                    job: Some(SchedulerJobConfig {
                        schedule: "1 0 * * * *".to_string(),
                        retry_strategy: None,
                        notifications: Some(true),
                    }),
                },
            })
            .await?;
        api.trackers()
            .update_tracker_job(tracker.id, Some(job_id))
            .await?;

        let job = TrackersTriggerJob::try_resume(
            api.clone(),
            mock_scheduler_job(job_id, SchedulerJob::TrackersTrigger, "0 0 * * * *"),
        )
        .await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert_eq!(
            unscheduled_trackers,
            vec![Tracker {
                job_id: None,
                ..tracker
            }]
        );

        assert!(api
            .trackers()
            .get_tracker_by_job_id(job_id)
            .await?
            .is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn removes_job_if_tracker_no_longer_has_schedule(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(TrackerCreateParams {
                name: "tracker".to_string(),
                url: "https://localhost:1234/my/app?q=2".parse()?,
                target: Default::default(),
                config: TrackerConfig {
                    revisions: 4,
                    extractor: Default::default(),
                    headers: Default::default(),
                    job: None,
                },
            })
            .await?;
        api.trackers()
            .update_tracker_job(tracker.id, Some(job_id))
            .await?;

        let job = TrackersTriggerJob::try_resume(
            api.clone(),
            mock_scheduler_job(job_id, SchedulerJob::TrackersTrigger, "0 0 * * * *"),
        )
        .await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(api
            .trackers()
            .get_tracker(tracker.id)
            .await?
            .unwrap()
            .job_id
            .is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn removes_job_if_tracker_no_longer_has_revisions(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
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
            })
            .await?;
        api.trackers()
            .update_tracker_job(tracker.id, Some(job_id))
            .await?;

        let job = TrackersTriggerJob::try_resume(
            api.clone(),
            mock_scheduler_job(job_id, SchedulerJob::TrackersTrigger, "0 0 * * * *"),
        )
        .await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(api
            .trackers()
            .get_tracker(tracker.id)
            .await?
            .unwrap()
            .job_id
            .is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn removes_job_if_tracker_no_longer_exists(pool: PgPool) -> anyhow::Result<()> {
        let api = Arc::new(mock_api(pool).await?);

        let job_id = uuid!("00000000-0000-0000-0000-000000000000");

        // Create tracker and tracker job.
        let job = TrackersTriggerJob::try_resume(
            api.clone(),
            mock_scheduler_job(job_id, SchedulerJob::TrackersTrigger, "0 0 * * * *"),
        )
        .await?;
        assert!(job.is_none());

        let job = TrackersTriggerJob::try_resume(
            api.clone(),
            mock_scheduler_job(job_id, SchedulerJob::TrackersTrigger, "0 0 * * * *"),
        )
        .await?;
        assert!(job.is_none());

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());

        assert!(api
            .trackers()
            .get_tracker_by_job_id(job_id)
            .await?
            .is_none());
        assert!(api
            .trackers()
            .get_tracker_by_job_id(job_id)
            .await?
            .is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn marks_job_as_stopped_when_run(pool: PgPool) -> anyhow::Result<()> {
        let mut scheduler = mock_scheduler(&pool).await?;
        let api = Arc::new(mock_api(pool).await?);

        let trigger_job_id = scheduler
            .add(TrackersTriggerJob::create(api.clone(), "1/1 * * * * *").await?)
            .await?;

        // Start scheduler and wait for a few seconds, then stop it.
        scheduler.start().await?;

        while !mock_get_scheduler_job(&api.db, trigger_job_id)
            .await?
            .and_then(|job| job.stopped)
            .unwrap_or_default()
        {}

        scheduler.shutdown().await?;

        Ok(())
    }
}
