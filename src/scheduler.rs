mod api_ext;
mod cron_ext;
mod database_ext;
mod job_ext;
mod scheduler_job;
mod scheduler_job_metadata;
mod scheduler_jobs;

use anyhow::anyhow;
use futures::{StreamExt, pin_mut};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::RwLock;
use tokio_cron_scheduler::{
    JobScheduler, PostgresMetadataStore, PostgresNotificationStore, PostgresStore, SimpleJobCode,
    SimpleNotificationCode,
};
use tracing::{debug, error, warn};

pub use self::{
    cron_ext::CronExt, scheduler_job::SchedulerJob, scheduler_job_metadata::SchedulerJobMetadata,
};
use crate::{
    api::Api,
    network::DnsResolver,
    scheduler::scheduler_jobs::{TasksRunJob, TrackersRunJob, TrackersScheduleJob},
    server::SchedulerStatus,
};

/// Defines a maximum number of jobs that can be retrieved from the database at once.
const MAX_JOBS_PAGE_SIZE: usize = 1000;

/// Defines a name of the table that stores scheduler jobs.
const SCHEDULER_JOBS_TABLE: &str = "scheduler_jobs";

/// Defines a name of the table that stores scheduler notifications.
const SCHEDULER_NOTIFICATIONS_TABLE: &str = "scheduler_notifications";

/// Defines a name of the table that stores scheduler notification states.
const SCHEDULER_NOTIFICATION_STATES_TABLE: &str = "scheduler_notification_states";

/// The scheduler is responsible for scheduling and executing jobs.
pub struct Scheduler<DR: DnsResolver> {
    pub inner_scheduler: JobScheduler,
    pub api: Arc<Api<DR>>,
}

impl<DR: DnsResolver> Scheduler<DR> {
    /// Starts the scheduler resuming existing jobs and adding new ones.
    pub async fn start(api: Arc<Api<DR>>) -> anyhow::Result<Self> {
        let store_db_config = &api.config.db;
        let store = Arc::new(RwLock::new(PostgresStore::Created(format!(
            "host={} port={} dbname={}{}",
            store_db_config.host,
            store_db_config.port,
            store_db_config.name,
            if let Some(password) = &store_db_config.password {
                format!(" user={} password={password}", store_db_config.username)
            } else {
                format!(" user={}", store_db_config.username)
            }
        ))));
        let metadata_store = PostgresMetadataStore {
            store: store.clone(),
            init_tables: false,
            table: SCHEDULER_JOBS_TABLE.to_string(),
        };

        let is_scheduler_enabled = api.config.scheduler.enabled;
        let scheduler = Self {
            inner_scheduler: JobScheduler::new_with_storage_and_code(
                Box::new(metadata_store),
                Box::new(PostgresNotificationStore {
                    store,
                    init_tables: false,
                    table: SCHEDULER_NOTIFICATIONS_TABLE.to_string(),
                    states_table: SCHEDULER_NOTIFICATION_STATES_TABLE.to_string(),
                }),
                Box::<SimpleJobCode>::default(),
                Box::<SimpleNotificationCode>::default(),
                1000,
            )
            .await?,
            api,
        };

        if !is_scheduler_enabled {
            warn!(
                "Scheduler is disabled – existing jobs won’t be resumed, and new jobs won’t be scheduled."
            );
            return Ok(scheduler);
        }

        // First, try to resume existing jobs.
        let resumed_unique_jobs = scheduler.resume().await?;
        if !resumed_unique_jobs.contains(&SchedulerJob::TrackersSchedule) {
            scheduler
                .inner_scheduler
                .add(TrackersScheduleJob::create(scheduler.api.clone())?)
                .await?;
        }

        if !resumed_unique_jobs.contains(&SchedulerJob::TasksRun) {
            scheduler
                .inner_scheduler
                .add(TasksRunJob::create(scheduler.api.clone())?)
                .await?;
        }

        scheduler.inner_scheduler.start().await?;
        Ok(scheduler)
    }

    /// Resumes existing jobs.
    async fn resume(&self) -> anyhow::Result<HashSet<SchedulerJob>> {
        let db = &self.api.db;
        let jobs = db.get_scheduler_jobs(MAX_JOBS_PAGE_SIZE);
        pin_mut!(jobs);

        // Track jobs for the job types that should be scheduled only once.
        let mut unique_resumed_jobs = HashSet::new();
        while let Some(job_data) = jobs.next().await {
            let job_data = job_data?;
            let job_id = job_data.id;
            let job_meta = job_data
                .extra
                .as_ref()
                .ok_or_else(|| anyhow!("Job `{job_data:?}` doesnt have extra data"))
                .and_then(|extra| SchedulerJobMetadata::try_from(extra.as_ref()));
            let job_meta = match job_meta {
                Ok(job_meta) if unique_resumed_jobs.contains(&job_meta.job_type) => {
                    // There can only be one job of each type. If we detect that there are multiple, we log
                    // a warning and remove the job, keeping only the first one.
                    error!(
                        "Found multiple jobs of type `{:?}`. All duplicated jobs except for the first one will be removed.",
                        job_meta.job_type
                    );
                    self.inner_scheduler.remove(&job_id).await?;
                    continue;
                }
                Err(err) => {
                    // We don't fail here, because we want to gracefully handle the legacy jobs.
                    error!(
                        "Failed to deserialize job type for job `{job_data:?}`: {err:?}. The job will be removed."
                    );
                    self.inner_scheduler.remove(&job_id).await?;
                    continue;
                }
                Ok(job_meta) => job_meta,
            };

            // First try to resume the job, and if it's not possible, the job will be removed and
            // re-scheduled at a later step if needed.
            let job = match &job_meta.job_type {
                SchedulerJob::TasksRun => TasksRunJob::try_resume(self.api.clone(), job_data)?,
                SchedulerJob::TrackersSchedule => {
                    TrackersScheduleJob::try_resume(self.api.clone(), job_data)?
                }
                SchedulerJob::TrackersRun => {
                    TrackersRunJob::try_resume(self.api.clone(), job_data).await?
                }
            };

            match job {
                Some(job) => {
                    debug!(job.id = %job_id, "Resumed job (`{:?}`).", job_meta.job_type);
                    self.inner_scheduler.add(job).await?;

                    if job_meta.job_type.is_unique() {
                        unique_resumed_jobs.insert(job_meta.job_type);
                    }
                }
                None => {
                    warn!(
                        job.id = %job_id,
                        "Failed to resume job (`{:?}`). The job will be removed and re-scheduled if needed.",
                        job_meta.job_type
                    );
                    self.inner_scheduler.remove(&job_id).await?;
                }
            }
        }

        Ok(unique_resumed_jobs)
    }

    /// Returns the status of the scheduler.
    pub async fn status(&mut self) -> anyhow::Result<SchedulerStatus> {
        match self.inner_scheduler.time_till_next_job().await {
            Ok(time_till_next_job) => Ok(SchedulerStatus {
                operational: self.api.config.scheduler.enabled,
                time_till_next_job,
            }),
            Err(err) => {
                error!(error = %err, "Failed to get scheduler status.");
                Ok(SchedulerStatus {
                    operational: false,
                    time_till_next_job: None,
                })
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    pub use super::database_ext::tests::{
        RawSchedulerJobStoredData, mock_get_scheduler_job, mock_upsert_scheduler_job,
    };
    use crate::{
        config::{Config, DatabaseConfig},
        scheduler::{Scheduler, SchedulerJobMetadata, scheduler_job::SchedulerJob},
        tests::{TrackerCreateParamsBuilder, mock_api_with_config, mock_config},
    };
    use anyhow::anyhow;
    use futures::StreamExt;
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use std::{env, sync::Arc, vec::Vec};
    use tokio::sync::RwLock;
    use tokio_cron_scheduler::{
        JobScheduler, PostgresMetadataStore, PostgresNotificationStore, PostgresStore,
        SimpleJobCode, SimpleNotificationCode,
    };
    use uuid::{Uuid, uuid};

    pub async fn mock_scheduler(pool: &PgPool) -> anyhow::Result<JobScheduler> {
        let connect_options = pool.connect_options();
        let store = Arc::new(RwLock::new(PostgresStore::Created(format!(
            "host={} port={} dbname={} {}",
            connect_options.get_host(),
            connect_options.get_port(),
            connect_options
                .get_database()
                .ok_or_else(|| anyhow!("Database name is not available"))?,
            if let Ok(password) = env::var("DATABASE_PASSWORD") {
                format!(
                    "user={} password={password}",
                    connect_options.get_username()
                )
            } else {
                format!("user={}", connect_options.get_username())
            }
        ))));

        let metadata_store = PostgresMetadataStore {
            store: store.clone(),
            init_tables: false,
            table: "scheduler_jobs".to_string(),
        };

        Ok(JobScheduler::new_with_storage_and_code(
            Box::new(metadata_store),
            Box::new(PostgresNotificationStore {
                store,
                init_tables: false,
                table: "scheduler_notifications".to_string(),
                states_table: "scheduler_notification_states".to_string(),
            }),
            Box::<SimpleJobCode>::default(),
            Box::<SimpleNotificationCode>::default(),
            1000,
        )
        .await?)
    }

    async fn mock_scheduler_config(pool: &PgPool) -> anyhow::Result<Config> {
        let connect_options = pool.connect_options();
        Ok(Config {
            db: DatabaseConfig {
                name: connect_options
                    .get_database()
                    .ok_or_else(|| anyhow!("Database name is not available"))?
                    .to_string(),
                host: connect_options.get_host().to_string(),
                port: connect_options.get_port(),
                username: connect_options.get_username().to_string(),
                password: env::var("DATABASE_PASSWORD").ok(),
                ..Default::default()
            },
            ..mock_config()?
        })
    }

    pub fn mock_scheduler_job(
        job_id: Uuid,
        job_type: SchedulerJob,
        schedule: impl Into<String>,
    ) -> RawSchedulerJobStoredData {
        RawSchedulerJobStoredData {
            id: job_id,
            job_type: 0,
            count: Some(0),
            last_tick: None,
            next_tick: Some(12),
            ran: Some(false),
            stopped: Some(false),
            schedule: Some(schedule.into()),
            repeating: None,
            last_updated: None,
            extra: Some(Vec::try_from(&SchedulerJobMetadata::new(job_type)).unwrap()),
            time_offset_seconds: Some(0),
            repeated_every: None,
        }
    }

    #[sqlx::test]
    async fn can_resume_jobs(pool: PgPool) -> anyhow::Result<()> {
        let mock_config = mock_scheduler_config(&pool).await?;
        let api = Arc::new(mock_api_with_config(pool, mock_config).await?);

        let trackers_run_job_id = uuid!("00000000-0000-0000-0000-000000000001");
        let trackers_schedule_job_id = uuid!("00000000-0000-0000-0000-000000000002");
        let tasks_run_job_id = uuid!("00000000-0000-0000-0000-000000000003");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-one")
                    .with_schedule("1 2 3 4 5 6")
                    .build(),
            )
            .await?;
        api.trackers()
            .set_tracker_job(tracker.id, trackers_run_job_id)
            .await?;

        // Add job registrations.
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(
                trackers_run_job_id,
                SchedulerJob::TrackersRun,
                "1 2 3 4 5 6",
            ),
        )
        .await?;
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(
                trackers_schedule_job_id,
                SchedulerJob::TrackersSchedule,
                "0 * 0 * * *",
            ),
        )
        .await?;
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(tasks_run_job_id, SchedulerJob::TasksRun, "0 * 1 * * *"),
        )
        .await?;

        let mut scheduler = Scheduler::start(api.clone()).await?;
        assert!(scheduler.inner_scheduler.inited().await);

        assert!(
            scheduler
                .inner_scheduler
                .next_tick_for_job(trackers_run_job_id)
                .await?
                .is_some()
        );
        assert!(
            scheduler
                .inner_scheduler
                .next_tick_for_job(trackers_schedule_job_id)
                .await?
                .is_some()
        );
        assert!(
            scheduler
                .inner_scheduler
                .next_tick_for_job(tasks_run_job_id)
                .await?
                .is_some()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn schedules_unique_jobs_if_not_started(pool: PgPool) -> anyhow::Result<()> {
        let mock_config = mock_scheduler_config(&pool).await?;
        let api = Arc::new(mock_api_with_config(pool, mock_config).await?);
        Scheduler::start(api.clone()).await?;

        let jobs = api.db.get_scheduler_jobs(10).collect::<Vec<_>>().await;
        assert_eq!(jobs.len(), 2);

        let mut jobs = jobs
            .into_iter()
            .map(|job_result| job_result.map(|job| (job.job_type, job.extra, job.schedule)))
            .collect::<anyhow::Result<Vec<(_, _, _)>>>()?;
        jobs.sort_by(|job_a, job_b| job_a.1.cmp(&job_b.1));

        assert_debug_snapshot!(jobs, @r###"
        [
            (
                0,
                Some(
                    [
                        0,
                        0,
                        0,
                    ],
                ),
                Some(
                    "0 * 1 * * *",
                ),
            ),
            (
                0,
                Some(
                    [
                        1,
                        0,
                        0,
                    ],
                ),
                Some(
                    "0 * 0 * * *",
                ),
            ),
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn schedules_unique_jobs_if_cannot_resume(pool: PgPool) -> anyhow::Result<()> {
        let mock_config = mock_scheduler_config(&pool).await?;
        let api = Arc::new(mock_api_with_config(pool, mock_config).await?);

        let trackers_schedule_job_id = uuid!("00000000-0000-0000-0000-000000000001");
        let tasks_run_job_id = uuid!("00000000-0000-0000-0000-000000000002");

        // Add job registration.
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(
                trackers_schedule_job_id,
                SchedulerJob::TrackersSchedule,
                // Different schedule - every hour, not every minute.
                "0 0 * * * * *",
            ),
        )
        .await?;
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(
                tasks_run_job_id,
                SchedulerJob::TasksRun,
                // Different schedule - every day, not every minute.
                "0 0 0 * * * *",
            ),
        )
        .await?;

        Scheduler::start(api.clone()).await?;

        // Old jobs should have been removed.
        assert!(
            mock_get_scheduler_job(&api.db, trackers_schedule_job_id)
                .await?
                .is_none()
        );
        assert!(
            mock_get_scheduler_job(&api.db, tasks_run_job_id)
                .await?
                .is_none()
        );

        let jobs = api.db.get_scheduler_jobs(10).collect::<Vec<_>>().await;
        assert_eq!(jobs.len(), 2);

        let mut jobs = jobs
            .into_iter()
            .map(|job_result| job_result.map(|job| (job.job_type, job.extra, job.schedule)))
            .collect::<anyhow::Result<Vec<(_, _, _)>>>()?;
        jobs.sort_by(|job_a, job_b| job_a.1.cmp(&job_b.1));

        assert_debug_snapshot!(jobs, @r###"
        [
            (
                0,
                Some(
                    [
                        0,
                        0,
                        0,
                    ],
                ),
                Some(
                    "0 * 1 * * *",
                ),
            ),
            (
                0,
                Some(
                    [
                        1,
                        0,
                        0,
                    ],
                ),
                Some(
                    "0 * 0 * * *",
                ),
            ),
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn doesnt_resume_jobs_if_disabled(pool: PgPool) -> anyhow::Result<()> {
        let mut mock_config = mock_scheduler_config(&pool).await?;
        mock_config.scheduler.enabled = false;
        let api = Arc::new(mock_api_with_config(pool, mock_config).await?);

        let trackers_run_job_id = uuid!("00000000-0000-0000-0000-000000000001");
        let trackers_schedule_job_id = uuid!("00000000-0000-0000-0000-000000000002");
        let tasks_run_job_id = uuid!("00000000-0000-0000-0000-000000000003");

        // Create tracker and tracker job.
        let tracker = api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker-one")
                    .with_schedule("1 2 3 4 5 6")
                    .build(),
            )
            .await?;
        api.trackers()
            .set_tracker_job(tracker.id, trackers_run_job_id)
            .await?;

        // Add job registrations.
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(
                trackers_run_job_id,
                SchedulerJob::TrackersRun,
                "1 2 3 4 5 6",
            ),
        )
        .await?;
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(
                trackers_schedule_job_id,
                SchedulerJob::TrackersSchedule,
                "0 * 0 * * *",
            ),
        )
        .await?;
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(tasks_run_job_id, SchedulerJob::TasksRun, "0 * 1 * * *"),
        )
        .await?;

        let scheduler = Scheduler::start(api.clone()).await?;
        assert!(!scheduler.inner_scheduler.inited().await);

        Ok(())
    }

    #[sqlx::test]
    async fn schedules_return_status(pool: PgPool) -> anyhow::Result<()> {
        let mock_config = mock_scheduler_config(&pool).await?;
        let api = Arc::new(mock_api_with_config(pool, mock_config).await?);
        let mut scheduler = Scheduler::start(api.clone()).await?;

        let jobs = api.db.get_scheduler_jobs(10).collect::<Vec<_>>().await;
        assert_eq!(jobs.len(), 2);

        let status = scheduler.status().await?;
        assert!(status.time_till_next_job.is_some());
        assert!(status.operational);

        Ok(())
    }
}
