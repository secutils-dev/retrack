use crate::{
    api::Api,
    network::{DnsResolver, EmailTransport},
    scheduler::SchedulerJobMetadata,
};
use uuid::Uuid;

pub struct SchedulerApiExt<'a, DR: DnsResolver, ET: EmailTransport> {
    api: &'a Api<DR, ET>,
}

impl<'a, DR: DnsResolver, ET: EmailTransport> SchedulerApiExt<'a, DR, ET> {
    /// Creates Scheduler API.
    pub fn new(api: &'a Api<DR, ET>) -> Self {
        Self { api }
    }

    /// Retrieves metadata for the scheduler job with the specified ID.
    pub async fn get_job_meta(&self, job_id: Uuid) -> anyhow::Result<Option<SchedulerJobMetadata>> {
        self.api.db.get_scheduler_job_meta(job_id).await
    }

    /// Sets metadata for the scheduler job with the specified ID.
    pub async fn set_job_meta(
        &self,
        job_id: Uuid,
        meta: &SchedulerJobMetadata,
    ) -> anyhow::Result<()> {
        self.api.db.update_scheduler_job_meta(job_id, meta).await
    }
}

impl<DR: DnsResolver, ET: EmailTransport> Api<DR, ET> {
    /// Returns an API to work with scheduler jobs.
    pub fn scheduler(&self) -> SchedulerApiExt<DR, ET> {
        SchedulerApiExt::new(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{database_ext::RawSchedulerJobStoredData, SchedulerJob, SchedulerJobMetadata},
        tests::{mock_api, mock_upsert_scheduler_job},
    };
    use sqlx::PgPool;
    use uuid::uuid;

    #[sqlx::test]
    async fn properly_sets_and_gets_job_meta(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let scheduler = api.scheduler();

        let job_id = uuid!("67e55044-10b1-426f-9247-bb680e5fe0c8");
        let job = RawSchedulerJobStoredData {
            id: job_id,
            last_updated: Some(946720800),
            last_tick: Some(946720700),
            next_tick: Some(946720900),
            count: Some(3),
            job_type: 3,
            extra: Some(Vec::try_from(&SchedulerJobMetadata::new(
                SchedulerJob::TasksRun,
            ))?),
            ran: Some(true),
            stopped: Some(false),
            schedule: None,
            repeating: None,
            time_offset_seconds: Some(0),
            repeated_every: None,
        };

        mock_upsert_scheduler_job(&api.db, &job).await?;
        assert_eq!(
            scheduler.get_job_meta(job_id).await?,
            Some(SchedulerJobMetadata::new(SchedulerJob::TasksRun))
        );

        scheduler
            .set_job_meta(
                job_id,
                &SchedulerJobMetadata {
                    job_type: SchedulerJob::TrackersRun,
                    retry_attempt: 5,
                    is_running: true,
                },
            )
            .await?;
        assert_eq!(
            scheduler.get_job_meta(job_id).await?,
            Some(SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                retry_attempt: 5,
                is_running: true,
            })
        );

        Ok(())
    }
}
