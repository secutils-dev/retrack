mod raw_tracker;
mod raw_tracker_data_revision;

use crate::{
    database::Database,
    error::Error as RetrackError,
    scheduler::SchedulerJobMetadata,
    trackers::{
        database_ext::raw_tracker_data_revision::RawTrackerDataRevision, Tracker,
        TrackerDataRevision,
    },
};
use anyhow::{anyhow, bail};
use async_stream::try_stream;
use futures::Stream;
use raw_tracker::RawTracker;
use sqlx::{error::ErrorKind as SqlxErrorKind, query, query_as, Pool, Postgres};
use time::OffsetDateTime;
use uuid::Uuid;

/// A database extension for the trackers-related operations.
pub struct TrackersDatabaseExt<'pool> {
    pool: &'pool Pool<Postgres>,
}

impl<'pool> TrackersDatabaseExt<'pool> {
    pub fn new(pool: &'pool Pool<Postgres>) -> Self {
        Self { pool }
    }

    /// Retrieves all trackers.
    pub async fn get_trackers(&self) -> anyhow::Result<Vec<Tracker>> {
        let raw_trackers = query_as!(
            RawTracker,
            r#"
SELECT id, name, url, target, config, created_at, job_id, job_needed
FROM trackers
ORDER BY created_at
                "#
        )
        .fetch_all(self.pool)
        .await?;

        let mut trackers = vec![];
        for raw_tracker in raw_trackers {
            trackers.push(Tracker::try_from(raw_tracker)?);
        }

        Ok(trackers)
    }

    /// Retrieves tracker with the specified ID.
    pub async fn get_tracker(&self, id: Uuid) -> anyhow::Result<Option<Tracker>> {
        query_as!(
            RawTracker,
            r#"
    SELECT id, name, url, target, config, created_at, job_id, job_needed
    FROM trackers
    WHERE id = $1
                    "#,
            id
        )
        .fetch_optional(self.pool)
        .await?
        .map(Tracker::try_from)
        .transpose()
    }

    /// Inserts tracker.
    pub async fn insert_tracker(&self, tracker: &Tracker) -> anyhow::Result<()> {
        let raw_tracker = RawTracker::try_from(tracker)?;
        let result = query!(
            r#"
    INSERT INTO trackers (id, name, url, target, config, created_at, job_needed, job_id)
    VALUES ( $1, $2, $3, $4, $5, $6, $7, $8 )
            "#,
            raw_tracker.id,
            raw_tracker.name,
            raw_tracker.url,
            raw_tracker.target,
            raw_tracker.config,
            raw_tracker.created_at,
            raw_tracker.job_needed,
            raw_tracker.job_id,
        )
        .execute(self.pool)
        .await;

        if let Err(err) = result {
            bail!(match err.as_database_error() {
                Some(database_error) if database_error.is_unique_violation() => {
                    RetrackError::client_with_root_cause(anyhow!(err).context(format!(
                        "Tracker with such name ('{}') or id ('{}') already exists.",
                        tracker.name, tracker.id
                    )))
                }
                _ => RetrackError::from(anyhow!(err).context(format!(
                    "Couldn't create tracker ('{}') due to unknown reason.",
                    tracker.name
                ))),
            });
        }

        Ok(())
    }

    /// Updates tracker.
    pub async fn update_tracker(&self, tracker: &Tracker) -> anyhow::Result<()> {
        let raw_tracker = RawTracker::try_from(tracker)?;
        let result = query!(
            r#"
UPDATE trackers
SET name = $2, url = $3, target = $4, config = $5, job_needed = $6, job_id = $7
WHERE id = $1
        "#,
            raw_tracker.id,
            raw_tracker.name,
            raw_tracker.url,
            raw_tracker.target,
            raw_tracker.config,
            raw_tracker.job_needed,
            raw_tracker.job_id
        )
        .execute(self.pool)
        .await;

        match result {
            Ok(result) => {
                if result.rows_affected() == 0 {
                    bail!(RetrackError::client(format!(
                        "Tracker ('{}') doesn't exist.",
                        tracker.name
                    )));
                }
            }
            Err(err) => {
                bail!(RetrackError::from(anyhow!(err).context(format!(
                    "Couldn't update tracker ('{}') due to unknown reason.",
                    tracker.name
                ))));
            }
        }

        Ok(())
    }

    /// Removes tracker with the specified ID.
    pub async fn remove_tracker(&self, id: Uuid) -> anyhow::Result<()> {
        query!(
            r#"
    DELETE FROM trackers
    WHERE id = $1
                    "#,
            id
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Retrieves all tracked data for the specified tracker.
    pub async fn get_tracker_data(
        &self,
        tracker_id: Uuid,
    ) -> anyhow::Result<Vec<TrackerDataRevision>> {
        let raw_revisions = query_as!(
            RawTrackerDataRevision,
            r#"
SELECT data.id, data.tracker_id, data.data, data.created_at
FROM trackers_data as data
INNER JOIN trackers
ON data.tracker_id = trackers.id
WHERE data.tracker_id = $1
ORDER BY data.created_at
                "#,
            tracker_id
        )
        .fetch_all(self.pool)
        .await?;

        let mut revisions = vec![];
        for raw_revision in raw_revisions {
            revisions.push(TrackerDataRevision::try_from(raw_revision)?);
        }

        Ok(revisions)
    }

    /// Removes tracker data.
    pub async fn clear_tracker_data(&self, tracker_id: Uuid) -> anyhow::Result<()> {
        query!(
            r#"
    DELETE FROM trackers_data
    WHERE tracker_id = $1
                    "#,
            tracker_id
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    // Inserts tracker revision.
    pub async fn insert_tracker_data_revision(
        &self,
        revision: &TrackerDataRevision,
    ) -> anyhow::Result<()> {
        let raw_revision = RawTrackerDataRevision::try_from(revision)?;
        let result = query!(
            r#"
    INSERT INTO trackers_data (id, tracker_id, data, created_at)
    VALUES ( $1, $2, $3, $4 )
            "#,
            raw_revision.id,
            raw_revision.tracker_id,
            raw_revision.data,
            raw_revision.created_at
        )
        .execute(self.pool)
        .await;

        if let Err(err) = result {
            let is_conflict_error = err
                .as_database_error()
                .map(|db_error| matches!(db_error.kind(), SqlxErrorKind::UniqueViolation))
                .unwrap_or_default();
            bail!(if is_conflict_error {
                RetrackError::client_with_root_cause(anyhow!(err).context(format!(
                    "Tracker revision ('{}') already exists.",
                    revision.id
                )))
            } else {
                RetrackError::from(anyhow!(err).context(format!(
                    "Couldn't create tracker revision ('{}') due to unknown reason.",
                    revision.id
                )))
            });
        }

        Ok(())
    }

    /// Removes tracker data revision.
    pub async fn remove_tracker_data_revision(
        &self,
        tracker_id: Uuid,
        id: Uuid,
    ) -> anyhow::Result<()> {
        query!(
            r#"
    DELETE FROM trackers_data
    WHERE tracker_id = $1 AND id = $2
                    "#,
            tracker_id,
            id
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Retrieves all trackers that need to be scheduled.
    pub async fn get_unscheduled_trackers(&self) -> anyhow::Result<Vec<Tracker>> {
        let raw_trackers = query_as!(
            RawTracker,
            r#"
SELECT id, name, url, target, config, created_at, job_needed, job_id
FROM trackers
WHERE job_needed = TRUE AND job_id IS NULL
ORDER BY created_at
                "#
        )
        .fetch_all(self.pool)
        .await?;

        let mut trackers = vec![];
        for raw_tracker in raw_trackers {
            let tracker = Tracker::try_from(raw_tracker)?;
            // Tracker without revisions shouldn't be scheduled.
            if tracker.config.revisions > 0 {
                trackers.push(tracker);
            }
        }

        Ok(trackers)
    }

    /// Retrieves all scheduled jobs from `scheduler_jobs` table that are in a `stopped` state.
    pub fn get_pending_trackers(
        &self,
        page_size: usize,
    ) -> impl Stream<Item = anyhow::Result<Tracker>> + '_ {
        let page_limit = page_size as i64;
        try_stream! {
            let mut last_created_at = OffsetDateTime::UNIX_EPOCH;
            let mut conn = self.pool.acquire().await?;
            loop {
                 let records = query!(
r#"
SELECT trackers.id, trackers.name, trackers.url, trackers.target, trackers.config,
       trackers.created_at, trackers.job_needed, trackers.job_id, jobs.extra
FROM trackers
INNER JOIN scheduler_jobs as jobs
ON trackers.job_id = jobs.id
WHERE jobs.stopped = true AND trackers.created_at > $1
ORDER BY trackers.created_at
LIMIT $2;
"#,
             last_created_at, page_limit
        )
            .fetch_all(&mut *conn)
            .await?;

                let is_last_page = records.len() < page_size;
                let now = OffsetDateTime::now_utc();
                for record in records {
                    last_created_at = record.created_at;

                    // Check if the tracker job is pending the retry attempt.
                    let job_meta = record.extra.map(|extra| SchedulerJobMetadata::try_from(extra.as_slice())).transpose()?;
                    if let Some(SchedulerJobMetadata { retry: Some(retry), .. }) = job_meta {
                        if retry.next_at > now {
                            continue;
                        }
                    }

                    yield Tracker::try_from(RawTracker {
                        id: record.id,
                        name: record.name,
                        url: record.url,
                        target: record.target,
                        config: record.config,
                        created_at: record.created_at,
                        job_needed: record.job_needed,
                        job_id: record.job_id,
                    })?;
                }

                if is_last_page {
                    break;
                }
            }
        }
    }

    /// Retrieves tracker by the specified job ID.
    pub async fn get_tracker_by_job_id(&self, job_id: Uuid) -> anyhow::Result<Option<Tracker>> {
        query_as!(
            RawTracker,
            r#"
    SELECT id, name, url, target, config, created_at, job_needed, job_id
    FROM trackers
    WHERE job_id = $1
                    "#,
            job_id
        )
        .fetch_optional(self.pool)
        .await?
        .map(Tracker::try_from)
        .transpose()
    }

    /// Updates tracker's job.
    pub async fn update_tracker_job(&self, id: Uuid, job_id: Option<Uuid>) -> anyhow::Result<()> {
        let result = query!(
            r#"
    UPDATE trackers
    SET job_id = $2
    WHERE id = $1
            "#,
            id,
            job_id
        )
        .execute(self.pool)
        .await?;

        if result.rows_affected() == 0 {
            bail!(RetrackError::client(format!(
                "Tracker ('{id}') doesn't exist.",
            )));
        }

        Ok(())
    }
}

impl Database {
    /// Returns a database extension for the trackers operations performed on.
    pub fn trackers(&self) -> TrackersDatabaseExt {
        TrackersDatabaseExt::new(&self.pool)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        database::Database,
        error::Error as RetrackError,
        scheduler::{SchedulerJob, SchedulerJobMetadata, SchedulerJobRetryState},
        tests::{
            mock_scheduler_job, mock_upsert_scheduler_job, to_database_error,
            MockWebPageTrackerBuilder, RawSchedulerJobStoredData,
        },
        trackers::{Tracker, TrackerDataRevision},
    };
    use futures::StreamExt;
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use std::{
        ops::{Add, Sub},
        time::Duration,
    };
    use time::OffsetDateTime;
    use uuid::{uuid, Uuid};

    fn create_data_revision(
        id: Uuid,
        tracker_id: Uuid,
        time_shift: i64,
    ) -> anyhow::Result<TrackerDataRevision> {
        Ok(TrackerDataRevision {
            id,
            tracker_id,
            created_at: OffsetDateTime::from_unix_timestamp(946720800 + time_shift)?,
            data: "some-data".to_string(),
        })
    }

    #[sqlx::test]
    async fn can_add_and_retrieve_trackers(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let mut trackers_list: Vec<Tracker> = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000004"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let tracker = trackers.get_tracker(trackers_list[0].id).await?.unwrap();
        assert_eq!(tracker, trackers_list.remove(0));

        let tracker = trackers.get_tracker(trackers_list[0].id).await?.unwrap();
        assert_eq!(tracker, trackers_list.remove(0));

        assert!(trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000005"))
            .await?
            .is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn correctly_handles_duplicated_trackers_on_insert(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "https://retrack.dev",
            3,
        )?
        .build();

        let trackers = db.trackers();
        trackers.insert_tracker(&tracker).await?;

        let insert_error = trackers
            .insert_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-other-name",
                    "https://retrack.dev",
                    3,
                )?
                .build(),
            )
            .await
            .unwrap_err()
            .downcast::<RetrackError>()
            .unwrap();
        assert_debug_snapshot!(
            insert_error.root_cause.to_string(),
            @r###""Tracker with such name ('some-other-name') or id ('00000000-0000-0000-0000-000000000001') already exists.""###
        );
        assert_debug_snapshot!(
            to_database_error(insert_error.root_cause)?.message(),
            @r###""duplicate key value violates unique constraint \"trackers_pkey\"""###
        );

        // Tracker with the same name, but different ID should not be allowed.
        let insert_error = trackers
            .insert_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000002"),
                    "some-name",
                    "https://retrack.dev",
                    3,
                )?
                .build(),
            )
            .await
            .unwrap_err()
            .downcast::<RetrackError>()
            .unwrap();
        assert_debug_snapshot!(
            insert_error.root_cause.to_string(),
            @r###""Tracker with such name ('some-name') or id ('00000000-0000-0000-0000-000000000002') already exists.""###
        );
        assert_debug_snapshot!(
            to_database_error(insert_error.root_cause)?.message(),
            @r###""duplicate key value violates unique constraint \"trackers_name_key\"""###
        );

        // Tracker with different name should be allowed.
        let insert_result = trackers
            .insert_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000003"),
                    "some-other-name",
                    "https://retrack.dev",
                    3,
                )?
                .build(),
            )
            .await;
        assert!(insert_result.is_ok());

        Ok(())
    }

    #[sqlx::test]
    async fn can_update_tracker(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers = db.trackers();
        trackers
            .insert_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-name",
                    "https://retrack.dev",
                    3,
                )?
                .build(),
            )
            .await?;
        trackers
            .insert_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000002"),
                    "some-other-name",
                    "https://retrack.dev",
                    3,
                )?
                .build(),
            )
            .await?;

        trackers
            .update_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-name-2",
                    "https://retrack.dev",
                    5,
                )?
                .build(),
            )
            .await?;
        trackers
            .update_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000002"),
                    "some-other-name-2",
                    "https://retrack.dev",
                    5,
                )?
                .build(),
            )
            .await?;

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .unwrap();
        assert_eq!(
            tracker,
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name-2",
                "https://retrack.dev",
                5,
            )?
            .build()
        );

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?
            .unwrap();
        assert_eq!(
            tracker,
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-other-name-2",
                "https://retrack.dev",
                5,
            )?
            .build()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn correctly_handles_non_existent_trackers_on_update(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let update_error = db
            .trackers()
            .update_tracker(
                &MockWebPageTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-name-2",
                    "https://retrack.dev",
                    5,
                )?
                .build(),
            )
            .await
            .unwrap_err()
            .downcast::<RetrackError>()
            .unwrap();
        assert_debug_snapshot!(
            update_error,
            @r###""Tracker ('some-name-2') doesn't exist.""###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_remove_trackers(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let mut trackers_list = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .unwrap();
        assert_eq!(tracker, trackers_list.remove(0));

        let tracker_2 = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?
            .unwrap();
        assert_eq!(tracker_2, trackers_list[0].clone());

        trackers
            .remove_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?;

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?;
        assert!(tracker.is_none());

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?
            .unwrap();
        assert_eq!(tracker, trackers_list.remove(0));

        trackers
            .remove_tracker(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?;

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?;
        assert!(tracker.is_none());

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?;
        assert!(tracker.is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn can_retrieve_all_trackers(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000004"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        assert_eq!(trackers.get_trackers().await?, trackers_list);

        Ok(())
    }

    #[sqlx::test]
    async fn can_add_and_retrieve_tracker_data(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        // No data yet.
        for tracker in trackers_list.iter() {
            assert!(trackers.get_tracker_data(tracker.id).await?.is_empty());
        }

        let mut revisions = vec![
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000001"),
                trackers_list[0].id,
                0,
            )?,
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000002"),
                trackers_list[0].id,
                1,
            )?,
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000003"),
                trackers_list[1].id,
                0,
            )?,
        ];
        for revision in revisions.iter() {
            trackers.insert_tracker_data_revision(revision).await?;
        }

        let tracker_one_data = trackers.get_tracker_data(trackers_list[0].id).await?;
        assert_eq!(
            tracker_one_data,
            vec![revisions.remove(0), revisions.remove(0)]
        );

        let tracker_two_data = trackers.get_tracker_data(trackers_list[1].id).await?;
        assert_eq!(tracker_two_data, vec![revisions.remove(0)]);

        assert!(trackers
            .get_tracker_data(uuid!("00000000-0000-0000-0000-000000000004"))
            .await?
            .is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_remove_tracker_data(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let revisions = vec![
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000001"),
                trackers_list[0].id,
                0,
            )?,
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000002"),
                trackers_list[0].id,
                1,
            )?,
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000003"),
                trackers_list[1].id,
                0,
            )?,
        ];
        for revision in revisions.iter() {
            trackers.insert_tracker_data_revision(revision).await?;
        }

        let tracker_data = trackers.get_tracker_data(trackers_list[0].id).await?;
        assert_eq!(
            tracker_data,
            vec![revisions[0].clone(), revisions[1].clone()]
        );

        let tracker_data = trackers.get_tracker_data(trackers_list[1].id).await?;
        assert_eq!(tracker_data, vec![revisions[2].clone()]);

        // Remove one revision.
        trackers
            .remove_tracker_data_revision(trackers_list[0].id, revisions[0].id)
            .await?;

        let tracker_data = trackers.get_tracker_data(trackers_list[0].id).await?;
        assert_eq!(tracker_data, vec![revisions[1].clone()]);

        let tracker_data = trackers.get_tracker_data(trackers_list[1].id).await?;
        assert_eq!(tracker_data, vec![revisions[2].clone()]);

        // Remove the rest of revisions.
        trackers
            .remove_tracker_data_revision(trackers_list[0].id, revisions[1].id)
            .await?;
        trackers
            .remove_tracker_data_revision(trackers_list[1].id, revisions[2].id)
            .await?;

        assert!(trackers
            .get_tracker_data(trackers_list[0].id)
            .await?
            .is_empty());
        assert!(trackers
            .get_tracker_data(trackers_list[1].id)
            .await?
            .is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_clear_all_data_revisions_at_once(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let revisions = vec![
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000001"),
                trackers_list[0].id,
                0,
            )?,
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000002"),
                trackers_list[0].id,
                1,
            )?,
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000003"),
                trackers_list[1].id,
                0,
            )?,
        ];
        for revision in revisions.iter() {
            trackers.insert_tracker_data_revision(revision).await?;
        }

        let tracker_data = trackers.get_tracker_data(trackers_list[0].id).await?;
        assert_eq!(
            tracker_data,
            vec![revisions[0].clone(), revisions[1].clone()]
        );

        let tracker_data = trackers.get_tracker_data(trackers_list[1].id).await?;
        assert_eq!(tracker_data, vec![revisions[2].clone()]);

        // Clear all revisions.
        trackers.clear_tracker_data(trackers_list[0].id).await?;
        trackers.clear_tracker_data(trackers_list[1].id).await?;

        assert!(trackers
            .get_tracker_data(trackers_list[0].id)
            .await?
            .is_empty());
        assert!(trackers
            .get_tracker_data(trackers_list[1].id)
            .await?
            .is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_retrieve_all_unscheduled_trackers(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let mut trackers_list: Vec<Tracker> = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000006"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .with_schedule("* * * * *")
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000007"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .with_schedule("* * * * *")
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000008"),
                "some-name-3",
                "https://retrack.dev",
                3,
            )?
            .with_schedule("* * * * *")
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000009"),
                "some-name-4",
                "https://retrack.dev",
                3,
            )?
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000010"),
                "some-name-5",
                "https://retrack.dev",
                0,
            )?
            .with_schedule("* * * * *")
            .build(),
        ];

        let trackers = db.trackers();
        for (index, tracker) in trackers_list.iter_mut().enumerate() {
            tracker.created_at = tracker
                .created_at
                .checked_add(Duration::from_secs(index as u64).try_into()?)
                .unwrap();
            trackers.insert_tracker(tracker).await?;
        }

        assert_eq!(trackers.get_trackers().await?, trackers_list);

        let trackers = db.trackers();
        assert_eq!(
            trackers.get_unscheduled_trackers().await?,
            vec![
                trackers_list[0].clone(),
                trackers_list[1].clone(),
                trackers_list[2].clone()
            ]
        );

        trackers
            .update_tracker_job(
                trackers_list[1].id,
                Some(uuid!("00000000-0000-0000-0000-000000000002")),
            )
            .await?;
        assert_eq!(
            trackers.get_trackers().await?,
            vec![
                trackers_list[0].clone(),
                Tracker {
                    job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                    ..trackers_list[1].clone()
                },
                trackers_list[2].clone(),
                trackers_list[3].clone(),
                trackers_list[4].clone(),
            ]
        );
        assert_eq!(
            trackers.get_unscheduled_trackers().await?,
            vec![trackers_list[0].clone(), trackers_list[2].clone()]
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_retrieve_tracker_by_job_id(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = vec![
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                "https://retrack.dev",
                3,
            )?
            .with_schedule("* * * * *")
            .with_job_id(uuid!("00000000-0000-0000-0000-000000000011"))
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                "https://retrack.dev",
                3,
            )?
            .with_schedule("* * * * *")
            .with_job_id(uuid!("00000000-0000-0000-0000-000000000022"))
            .build(),
            MockWebPageTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name-3",
                "https://retrack.dev",
                3,
            )?
            .with_schedule("* * * * *")
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let trackers = db.trackers();
        assert_eq!(
            trackers
                .get_tracker_by_job_id(uuid!("00000000-0000-0000-0000-000000000011"))
                .await?,
            Some(trackers_list[0].clone())
        );
        assert_eq!(
            trackers
                .get_tracker_by_job_id(uuid!("00000000-0000-0000-0000-000000000022"))
                .await?,
            Some(trackers_list[1].clone())
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_update_trackers_job_id(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "https://retrack.dev",
            3,
        )?
        .with_schedule("* * * * *")
        .build();

        let trackers = db.trackers();
        trackers.insert_tracker(&tracker).await?;

        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            None
        );

        let trackers = db.trackers();
        trackers
            .update_tracker_job(
                tracker.id,
                Some(uuid!("00000000-0000-0000-0000-000000000011")),
            )
            .await?;
        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            Some(uuid!("00000000-0000-0000-0000-000000000011"))
        );

        trackers
            .update_tracker_job(
                tracker.id,
                Some(uuid!("00000000-0000-0000-0000-000000000022")),
            )
            .await?;
        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            Some(uuid!("00000000-0000-0000-0000-000000000022"))
        );

        Ok(())
    }

    #[sqlx::test]
    async fn fails_to_update_trackers_job_id_if_needed(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "https://retrack.dev",
            3,
        )?
        .build();

        let update_and_fail = |result: anyhow::Result<_>| -> RetrackError {
            result.unwrap_err().downcast::<RetrackError>().unwrap()
        };

        // Non-existent tracker
        let update_result = update_and_fail(
            db.trackers()
                .update_tracker_job(
                    tracker.id,
                    Some(uuid!("00000000-0000-0000-0000-000000000011")),
                )
                .await,
        );
        assert_eq!(
            update_result.to_string(),
            format!("Tracker ('{}') doesn't exist.", tracker.id)
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_return_tracker_with_pending_jobs(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let pending_trackers = db
            .trackers()
            .get_pending_trackers(10)
            .collect::<Vec<_>>()
            .await;
        assert!(pending_trackers.is_empty());

        for n in 0..=2 {
            let job = RawSchedulerJobStoredData {
                last_updated: Some(946720800 + n),
                last_tick: Some(946720700),
                next_tick: Some(946720900),
                ran: Some(true),
                count: Some(n as i32),
                stopped: Some(n != 1),
                ..mock_scheduler_job(
                    Uuid::parse_str(&format!("68e55044-10b1-426f-9247-bb680e5fe0c{}", n))?,
                    SchedulerJob::TrackersTrigger,
                    format!("{} 0 0 1 1 * *", n),
                )
            };

            mock_upsert_scheduler_job(&db, &job).await?;
        }

        for n in 0..=2 {
            db.trackers()
                .insert_tracker(
                    &MockWebPageTrackerBuilder::create(
                        Uuid::parse_str(&format!("78e55044-10b1-426f-9247-bb680e5fe0c{}", n))?,
                        format!("name_{}", n),
                        "https://retrack.dev",
                        3,
                    )?
                    .with_schedule("0 0 * * * *")
                    .with_job_id(Uuid::parse_str(&format!(
                        "68e55044-10b1-426f-9247-bb680e5fe0c{}",
                        n
                    ))?)
                    .build(),
                )
                .await?;
        }

        let pending_trackers = db
            .trackers()
            .get_pending_trackers(10)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(pending_trackers.len(), 2);

        Ok(())
    }

    #[sqlx::test]
    async fn can_return_tracker_with_pending_jobs_with_retry(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let pending_trackers = db
            .trackers()
            .get_pending_trackers(10)
            .collect::<Vec<_>>()
            .await;
        assert!(pending_trackers.is_empty());

        for n in 0..=2 {
            let job = RawSchedulerJobStoredData {
                last_updated: Some(946720800 + n),
                last_tick: Some(946720700),
                next_tick: Some(946720900),
                ran: Some(true),
                count: Some(n as i32),
                stopped: Some(n != 1),
                extra: Some(
                    if n == 2 {
                        SchedulerJobMetadata {
                            job_type: SchedulerJob::TrackersTrigger,
                            retry: Some(SchedulerJobRetryState {
                                attempts: 1,
                                next_at: OffsetDateTime::now_utc().add(Duration::from_secs(3600)),
                            }),
                        }
                    } else {
                        SchedulerJobMetadata::new(SchedulerJob::TrackersTrigger)
                    }
                    .try_into()?,
                ),
                ..mock_scheduler_job(
                    Uuid::parse_str(&format!("67e55044-10b1-426f-9247-bb680e5fe0c{}", n))?,
                    SchedulerJob::TrackersTrigger,
                    format!("{} 0 0 1 1 * *", n),
                )
            };

            mock_upsert_scheduler_job(&db, &job).await?;
        }

        for n in 0..=2 {
            db.trackers()
                .insert_tracker(
                    &MockWebPageTrackerBuilder::create(
                        Uuid::parse_str(&format!("77e55044-10b1-426f-9247-bb680e5fe0c{}", n))?,
                        format!("name_{}", n),
                        "https://retrack.dev",
                        3,
                    )?
                    .with_schedule("0 0 * * * *")
                    .with_job_id(Uuid::parse_str(&format!(
                        "67e55044-10b1-426f-9247-bb680e5fe0c{}",
                        n
                    ))?)
                    .build(),
                )
                .await?;
        }

        let mut pending_trackers = db
            .trackers()
            .get_pending_trackers(10)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(pending_trackers.len(), 1);

        let tracker = pending_trackers.remove(0)?;
        assert_eq!(tracker.id, uuid!("77e55044-10b1-426f-9247-bb680e5fe0c0"));
        assert_eq!(
            tracker.job_id,
            Some(uuid!("67e55044-10b1-426f-9247-bb680e5fe0c0"))
        );

        db.update_scheduler_job_meta(
            uuid!("67e55044-10b1-426f-9247-bb680e5fe0c2"),
            SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersTrigger,
                retry: Some(SchedulerJobRetryState {
                    attempts: 1,
                    next_at: OffsetDateTime::now_utc().sub(Duration::from_secs(3600)),
                }),
            },
        )
        .await?;

        let mut pending_trackers = db
            .trackers()
            .get_pending_trackers(10)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(pending_trackers.len(), 2);

        let tracker = pending_trackers.remove(0)?;
        assert_eq!(tracker.id, uuid!("77e55044-10b1-426f-9247-bb680e5fe0c0"));
        assert_eq!(
            tracker.job_id,
            Some(uuid!("67e55044-10b1-426f-9247-bb680e5fe0c0"))
        );

        let tracker = pending_trackers.remove(0)?;
        assert_eq!(tracker.id, uuid!("77e55044-10b1-426f-9247-bb680e5fe0c2"));
        assert_eq!(
            tracker.job_id,
            Some(uuid!("67e55044-10b1-426f-9247-bb680e5fe0c2"))
        );

        Ok(())
    }
}
