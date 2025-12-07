mod raw_tracker;
mod raw_tracker_data_revision;

use crate::{
    database::Database, error::Error as RetrackError,
    trackers::database_ext::raw_tracker_data_revision::RawTrackerDataRevision,
};
use anyhow::{anyhow, bail};
use raw_tracker::RawTracker;
use retrack_types::trackers::{Tracker, TrackerDataRevision};
use sqlx::{Pool, Postgres, error::ErrorKind as SqlxErrorKind, query, query_as};
use uuid::Uuid;

/// A database extension for the trackers-related operations.
pub struct TrackersDatabaseExt<'pool> {
    pool: &'pool Pool<Postgres>,
}

impl<'pool> TrackersDatabaseExt<'pool> {
    pub fn new(pool: &'pool Pool<Postgres>) -> Self {
        Self { pool }
    }

    /// Retrieves all trackers that have all specified tags. If the `tags` field is empty, all
    /// trackers are returned.
    pub async fn get_trackers(&self, tags: &[String]) -> anyhow::Result<Vec<Tracker>> {
        let raw_trackers = if tags.is_empty() {
            query_as!(
                RawTracker,
                r#"
SELECT id, name, enabled, config, tags, created_at, updated_at, job_id, job_needed
FROM trackers
ORDER BY updated_at
                "#
            )
            .fetch_all(self.pool)
            .await?
        } else {
            query_as!(
                RawTracker,
                r#"
SELECT id, name, enabled, config, tags, created_at, updated_at, job_id, job_needed
FROM trackers
WHERE tags @> $1
ORDER BY updated_at
                "#,
                tags
            )
            .fetch_all(self.pool)
            .await?
        };

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
    SELECT id, name, enabled, config, tags, created_at, updated_at, job_id, job_needed
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
    INSERT INTO trackers (id, name, enabled, config, tags, created_at, updated_at, job_needed, job_id)
    VALUES ( $1, $2, $3, $4, $5, $6, $7, $8, $9 )
            "#,
            raw_tracker.id,
            raw_tracker.name,
            raw_tracker.enabled,
            raw_tracker.config,
            &raw_tracker.tags,
            raw_tracker.created_at,
            raw_tracker.updated_at,
            raw_tracker.job_needed,
            raw_tracker.job_id,
        )
        .execute(self.pool)
        .await;

        if let Err(err) = result {
            bail!(match err.as_database_error() {
                Some(database_error) if database_error.is_unique_violation() => {
                    RetrackError::client_with_root_cause(anyhow!(err).context(format!(
                        "Tracker with such id ('{}') already exists.",
                        tracker.id
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
SET name = $2, enabled = $3, config = $4, tags = $5, updated_at = $6, job_needed = $7, job_id = $8
WHERE id = $1
        "#,
            raw_tracker.id,
            raw_tracker.name,
            raw_tracker.enabled,
            raw_tracker.config,
            &raw_tracker.tags,
            raw_tracker.updated_at,
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
    pub async fn remove_tracker(&self, id: Uuid) -> anyhow::Result<bool> {
        let result = query!(
            r#"
    DELETE FROM trackers
    WHERE id = $1
                    "#,
            id
        )
        .execute(self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Removes all trackers that have all specified tags. If the `tags` field is empty, all
    /// trackers are removed.
    pub async fn remove_trackers(&self, tags: &[String]) -> anyhow::Result<u64> {
        let result = if tags.is_empty() {
            query!(r#"DELETE FROM trackers"#).execute(self.pool).await?
        } else {
            query!(r#"DELETE FROM trackers WHERE tags @> $1"#, tags)
                .execute(self.pool)
                .await?
        };

        Ok(result.rows_affected())
    }

    /// Retrieves tracked data for the specified tracker sorted by creation date (desc).
    pub async fn get_tracker_data_revisions(
        &self,
        tracker_id: Uuid,
        size: usize,
    ) -> anyhow::Result<Vec<TrackerDataRevision>> {
        let raw_revisions = query_as!(
            RawTrackerDataRevision,
            r#"
SELECT data.id, data.tracker_id, data.data, data.created_at
FROM trackers_data as data
INNER JOIN trackers
ON data.tracker_id = trackers.id
WHERE data.tracker_id = $1
ORDER BY data.created_at DESC
LIMIT $2
                "#,
            tracker_id,
            size as i64
        )
        .fetch_all(self.pool)
        .await?;

        let mut revisions = vec![];
        for raw_revision in raw_revisions {
            revisions.push(TrackerDataRevision::try_from(raw_revision)?);
        }

        Ok(revisions)
    }

    /// Retrieves tracker data revision with the specified ID.
    pub async fn get_tracker_data_revision(
        &self,
        tracker_id: Uuid,
        id: Uuid,
    ) -> anyhow::Result<Option<TrackerDataRevision>> {
        query_as!(
            RawTrackerDataRevision,
            r#"
SELECT data.id, data.tracker_id, data.data, data.created_at
FROM trackers_data as data
INNER JOIN trackers
ON data.tracker_id = trackers.id
WHERE data.tracker_id = $1 AND data.id = $2
LIMIT 1
                    "#,
            tracker_id,
            id
        )
        .fetch_optional(self.pool)
        .await?
        .map(TrackerDataRevision::try_from)
        .transpose()
    }

    /// Inserts tracker revision.
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

    /// Removes tracker data revision with the specified ID.
    pub async fn remove_tracker_data_revision(
        &self,
        tracker_id: Uuid,
        id: Uuid,
    ) -> anyhow::Result<bool> {
        // Query is more complex than it can be, but it's to additional layer of protection against
        // accidental deletion of data.
        let result = query!(
            r#"
    DELETE FROM trackers_data as data
    USING trackers as t
    WHERE data.tracker_id = t.id AND t.id = $1 AND data.id = $2
                    "#,
            tracker_id,
            id
        )
        .execute(self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Removes all tracker data revisions.
    pub async fn clear_tracker_data_revisions(&self, tracker_id: Uuid) -> anyhow::Result<()> {
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

    /// Removes the oldest tracker data revisions that are beyond the specified limit.
    pub async fn enforce_tracker_data_revisions_limit(
        &self,
        tracker_id: Uuid,
        limit: usize,
    ) -> anyhow::Result<()> {
        query!(
            r#"
    DELETE FROM trackers_data USING (
        SELECT id FROM trackers_data
        WHERE tracker_id = $1
        ORDER BY created_at DESC
        OFFSET $2
    ) AS oldest_revisions
    WHERE trackers_data.id = oldest_revisions.id
                    "#,
            tracker_id,
            limit as i64
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Retrieves all trackers that need to be scheduled (either don't have associated job ID or
    /// job ID isn't valid).
    pub async fn get_trackers_to_schedule(&self) -> anyhow::Result<Vec<Tracker>> {
        let raw_trackers = query_as!(
            RawTracker,
            r#"
SELECT t.id, t.name, t.enabled, t.config, t.tags, t.created_at, t.updated_at, t.job_needed, t.job_id
FROM trackers as t
LEFT JOIN scheduler_jobs sj ON t.job_id = sj.id
WHERE t.job_needed = TRUE AND t.enabled = TRUE AND (t.job_id IS NULL OR sj.id IS NULL)
ORDER BY t.updated_at
                "#
        )
        .fetch_all(self.pool)
        .await?;

        raw_trackers
            .into_iter()
            .map(Tracker::try_from)
            .collect::<Result<_, _>>()
    }

    /// Retrieves tracker by the specified job ID.
    pub async fn get_tracker_by_job_id(&self, job_id: Uuid) -> anyhow::Result<Option<Tracker>> {
        query_as!(
            RawTracker,
            r#"
    SELECT id, name, enabled, config, tags, created_at, updated_at, job_needed, job_id
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
    /// Returns a database extension for the tracker operations.
    pub fn trackers(&self) -> TrackersDatabaseExt<'_> {
        TrackersDatabaseExt::new(&self.pool)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        database::Database,
        error::Error as RetrackError,
        scheduler::SchedulerJob,
        tests::{
            MockTrackerBuilder, mock_scheduler_job, mock_upsert_scheduler_job, to_database_error,
        },
    };
    use insta::assert_debug_snapshot;
    use retrack_types::trackers::{Tracker, TrackerDataRevision, TrackerDataValue};
    use serde_json::json;
    use sqlx::PgPool;
    use std::time::Duration;
    use time::OffsetDateTime;
    use uuid::{Uuid, uuid};

    fn create_data_revision(
        id: Uuid,
        tracker_id: Uuid,
        time_shift: i64,
    ) -> anyhow::Result<TrackerDataRevision> {
        Ok(TrackerDataRevision {
            id,
            tracker_id,
            created_at: OffsetDateTime::from_unix_timestamp(946720800 + time_shift)?,
            data: TrackerDataValue::new(json!("some-data")),
        })
    }

    #[sqlx::test]
    async fn can_add_and_retrieve_trackers(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let mut trackers_list: Vec<Tracker> = vec![
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000004"),
                "some-name-2",
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

        assert!(
            trackers
                .get_tracker(uuid!("00000000-0000-0000-0000-000000000005"))
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn correctly_handles_duplicated_trackers_on_insert(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .build();

        let trackers = db.trackers();
        trackers.insert_tracker(&tracker).await?;

        let insert_error = trackers
            .insert_tracker(
                &MockTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-other-name",
                    3,
                )?
                .build(),
            )
            .await
            .unwrap_err()
            .downcast::<RetrackError>()?;
        assert_debug_snapshot!(
            insert_error.root_cause.to_string(),
            @r###""Tracker with such id ('00000000-0000-0000-0000-000000000001') already exists.""###
        );
        assert_debug_snapshot!(
            to_database_error(insert_error.root_cause)?.message(),
            @r###""duplicate key value violates unique constraint \"trackers_pkey\"""###
        );

        // Tracker with the same name should be allowed.
        let insert_result = trackers
            .insert_tracker(
                &MockTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000002"),
                    "some-name",
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
                &MockTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-name",
                    3,
                )?
                .build(),
            )
            .await?;
        trackers
            .insert_tracker(
                &MockTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000002"),
                    "some-other-name",
                    3,
                )?
                .build(),
            )
            .await?;

        trackers
            .update_tracker(
                &MockTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-name-2",
                    5,
                )?
                .build(),
            )
            .await?;
        trackers
            .update_tracker(
                &MockTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000002"),
                    "some-other-name-2",
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
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name-2",
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
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-other-name-2",
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
                &MockTrackerBuilder::create(
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    "some-name-2",
                    5,
                )?
                .build(),
            )
            .await
            .unwrap_err()
            .downcast::<RetrackError>()?;
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
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
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

        // Non-existent tracker.
        assert!(
            !trackers
                .remove_tracker(uuid!("00000000-0000-0000-0000-000000000003"))
                .await?
        );

        // Existent tracker.
        assert!(
            trackers
                .remove_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
                .await?
        );

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?;
        assert!(tracker.is_none());

        let tracker = trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000002"))
            .await?
            .unwrap();
        assert_eq!(tracker, trackers_list.remove(0));

        assert!(
            trackers
                .remove_tracker(uuid!("00000000-0000-0000-0000-000000000002"))
                .await?
        );

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
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000004"),
                "some-name-2",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        assert_eq!(trackers.get_trackers(&[]).await?, trackers_list);

        Ok(())
    }

    #[sqlx::test]
    async fn can_bulk_remove_all_trackers(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = [
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000004"),
                "some-name-2",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        assert_eq!(trackers.remove_trackers(&[]).await?, 2);
        assert!(trackers.get_trackers(&[]).await?.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_retrieve_trackers_by_tags(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = vec![
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .with_tags(vec!["tag:1".to_string()])
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                3,
            )?
            .with_tags(vec!["tag:1".to_string(), "tag:2".to_string()])
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name-3",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        assert_eq!(trackers.get_trackers(&[]).await?, trackers_list);
        assert_eq!(
            trackers.get_trackers(&["tag:1".to_string()]).await?,
            vec![trackers_list[0].clone(), trackers_list[1].clone()]
        );
        assert_eq!(
            trackers.get_trackers(&["tag:2".to_string()]).await?,
            vec![trackers_list[1].clone()]
        );
        assert_eq!(
            trackers
                .get_trackers(&["tag:1".to_string(), "tag:2".to_string()])
                .await?,
            vec![trackers_list[1].clone()]
        );
        assert!(
            trackers
                .get_trackers(&[
                    "tag:1".to_string(),
                    "tag:2".to_string(),
                    "tag:3".to_string()
                ])
                .await?
                .is_empty()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_remove_trackers_by_tags(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = vec![
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .with_tags(vec!["tag:1".to_string()])
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                3,
            )?
            .with_tags(vec!["tag:1".to_string(), "tag:2".to_string()])
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name-3",
                3,
            )?
            .with_tags(vec!["tag:3".to_string()])
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000004"),
                "some-name-4",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        assert_eq!(trackers.get_trackers(&[]).await?, trackers_list);
        assert_eq!(trackers.remove_trackers(&["tag:1".to_string()]).await?, 2);
        assert_eq!(
            trackers.get_trackers(&[]).await?,
            vec![trackers_list[2].clone(), trackers_list[3].clone()]
        );

        assert_eq!(trackers.remove_trackers(&["tag:2".to_string()]).await?, 0);
        assert_eq!(
            trackers.get_trackers(&[]).await?,
            vec![trackers_list[2].clone(), trackers_list[3].clone()]
        );

        assert_eq!(trackers.remove_trackers(&["tag:3".to_string()]).await?, 1);
        assert_eq!(
            trackers.get_trackers(&[]).await?,
            vec![trackers_list[3].clone()]
        );

        assert_eq!(trackers.remove_trackers(&[]).await?, 1);
        assert!(trackers.get_trackers(&[]).await?.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_add_and_retrieve_tracker_data(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = [
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
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
            assert!(
                trackers
                    .get_tracker_data_revisions(tracker.id, 100)
                    .await?
                    .is_empty()
            );
        }

        let revisions = [
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

        assert_eq!(
            trackers
                .get_tracker_data_revision(trackers_list[0].id, revisions.first().unwrap().id)
                .await?,
            Some(revisions[0].clone())
        );
        assert_eq!(
            trackers
                .get_tracker_data_revision(trackers_list[0].id, revisions.get(1).unwrap().id)
                .await?,
            Some(revisions[1].clone())
        );
        assert!(
            trackers
                .get_tracker_data_revision(
                    trackers_list[0].id,
                    uuid!("00000000-0000-0000-0000-000000000003")
                )
                .await?
                .is_none()
        );

        assert_eq!(
            trackers
                .get_tracker_data_revision(trackers_list[1].id, revisions.get(2).unwrap().id)
                .await?,
            Some(revisions[2].clone())
        );
        assert!(
            trackers
                .get_tracker_data_revision(
                    trackers_list[1].id,
                    uuid!("00000000-0000-0000-0000-000000000002")
                )
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_retrieve_all_tracker_data_revisions(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = [
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
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
            assert!(
                trackers
                    .get_tracker_data_revisions(tracker.id, 100)
                    .await?
                    .is_empty()
            );
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

        let tracker_one_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 100)
            .await?;
        assert_eq!(
            tracker_one_data,
            vec![
                revisions.get(1).unwrap().clone(),
                revisions.first().unwrap().clone()
            ]
        );

        let tracker_one_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 2)
            .await?;
        assert_eq!(
            tracker_one_data,
            vec![revisions.get(1).unwrap().clone(), revisions.remove(0)]
        );

        let tracker_one_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 1)
            .await?;
        assert_eq!(tracker_one_data, vec![revisions.remove(0)]);

        let tracker_two_data = trackers
            .get_tracker_data_revisions(trackers_list[1].id, 100)
            .await?;
        assert_eq!(tracker_two_data, vec![revisions.remove(0)]);

        assert!(
            trackers
                .get_tracker_data_revisions(uuid!("00000000-0000-0000-0000-000000000004"), 100)
                .await?
                .is_empty()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_remove_tracker_data_revision(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = [
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let revisions = [
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

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 100)
            .await?;
        assert_eq!(
            tracker_data,
            vec![revisions[1].clone(), revisions[0].clone()]
        );

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[1].id, 100)
            .await?;
        assert_eq!(tracker_data, vec![revisions[2].clone()]);

        // Remove non-existent revision.
        assert!(
            !trackers
                .remove_tracker_data_revision(
                    trackers_list[0].id,
                    uuid!("00000000-0000-0000-0000-000000000004")
                )
                .await?
        );

        // Remove one revision.
        assert!(
            trackers
                .remove_tracker_data_revision(trackers_list[0].id, revisions[0].id)
                .await?
        );

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 100)
            .await?;
        assert_eq!(tracker_data, vec![revisions[1].clone()]);

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[1].id, 100)
            .await?;
        assert_eq!(tracker_data, vec![revisions[2].clone()]);

        // Remove the rest of the revisions.
        assert!(
            trackers
                .remove_tracker_data_revision(trackers_list[0].id, revisions[1].id)
                .await?
        );
        assert!(
            trackers
                .remove_tracker_data_revision(trackers_list[1].id, revisions[2].id)
                .await?
        );

        assert!(
            trackers
                .get_tracker_data_revisions(trackers_list[0].id, 100)
                .await?
                .is_empty()
        );
        assert!(
            trackers
                .get_tracker_data_revisions(trackers_list[1].id, 100)
                .await?
                .is_empty()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_clear_all_data_revisions_at_once(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = [
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let revisions = [
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

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 100)
            .await?;
        assert_eq!(
            tracker_data,
            vec![revisions[1].clone(), revisions[0].clone()]
        );

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[1].id, 100)
            .await?;
        assert_eq!(tracker_data, vec![revisions[2].clone()]);

        // Clear all revisions.
        trackers
            .clear_tracker_data_revisions(trackers_list[0].id)
            .await?;
        trackers
            .clear_tracker_data_revisions(trackers_list[1].id)
            .await?;

        assert!(
            trackers
                .get_tracker_data_revisions(trackers_list[0].id, 100)
                .await?
                .is_empty()
        );
        assert!(
            trackers
                .get_tracker_data_revisions(trackers_list[1].id, 100)
                .await?
                .is_empty()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_enforce_tracker_data_revisions_limit(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = [
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                3,
            )?
            .build(),
        ];

        let trackers = db.trackers();
        for tracker in trackers_list.iter() {
            trackers.insert_tracker(tracker).await?;
        }

        let revisions = [
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
                trackers_list[0].id,
                2,
            )?,
            create_data_revision(
                uuid!("00000000-0000-0000-0000-000000000004"),
                trackers_list[1].id,
                0,
            )?,
        ];
        for revision in revisions.iter() {
            trackers.insert_tracker_data_revision(revision).await?;
        }

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 100)
            .await?;
        assert_eq!(
            tracker_data,
            vec![
                revisions[2].clone(),
                revisions[1].clone(),
                revisions[0].clone()
            ]
        );

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[1].id, 100)
            .await?;
        assert_eq!(tracker_data, vec![revisions[3].clone()]);

        // No-op enforce.
        trackers
            .enforce_tracker_data_revisions_limit(trackers_list[0].id, 3)
            .await?;
        trackers
            .enforce_tracker_data_revisions_limit(trackers_list[1].id, 1)
            .await?;

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 100)
            .await?;
        assert_eq!(
            tracker_data,
            vec![
                revisions[2].clone(),
                revisions[1].clone(),
                revisions[0].clone()
            ]
        );

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[1].id, 100)
            .await?;
        assert_eq!(tracker_data, vec![revisions[3].clone()]);

        // Partial enforce.
        trackers
            .enforce_tracker_data_revisions_limit(trackers_list[0].id, 1)
            .await?;
        trackers
            .enforce_tracker_data_revisions_limit(trackers_list[1].id, 0)
            .await?;

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[0].id, 100)
            .await?;
        assert_eq!(tracker_data, vec![revisions[2].clone()]);

        let tracker_data = trackers
            .get_tracker_data_revisions(trackers_list[1].id, 100)
            .await?;
        assert!(tracker_data.is_empty());

        // Full removal.
        trackers
            .enforce_tracker_data_revisions_limit(trackers_list[0].id, 0)
            .await?;
        trackers
            .enforce_tracker_data_revisions_limit(trackers_list[1].id, 0)
            .await?;

        assert!(
            trackers
                .get_tracker_data_revisions(trackers_list[0].id, 100)
                .await?
                .is_empty()
        );
        assert!(
            trackers
                .get_tracker_data_revisions(trackers_list[1].id, 100)
                .await?
                .is_empty()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_retrieve_all_unscheduled_trackers(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let mut trackers_list: Vec<Tracker> = vec![
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000006"),
                "some-name",
                3,
            )?
            .with_schedule("* * * * *")
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000007"),
                "some-name-2",
                3,
            )?
            .with_schedule("* * * * *")
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000008"),
                "some-name-3",
                3,
            )?
            .with_schedule("* * * * *")
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000009"),
                "some-name-4",
                3,
            )?
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000010"),
                "some-name-5",
                0,
            )?
            .with_schedule("* * * * *")
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000011"),
                "some-name-6",
                3,
            )?
            .with_schedule("* * * * *")
            .disabled()
            .build(),
        ];

        let trackers = db.trackers();
        for (index, tracker) in trackers_list.iter_mut().enumerate() {
            tracker.updated_at = tracker
                .updated_at
                .checked_add(Duration::from_secs(index as u64).try_into()?)
                .unwrap();
            trackers.insert_tracker(tracker).await?;
        }

        assert_eq!(trackers.get_trackers(&[]).await?, trackers_list);

        let trackers = db.trackers();
        assert_eq!(
            trackers.get_trackers_to_schedule().await?,
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
            trackers.get_trackers(&[]).await?,
            vec![
                trackers_list[0].clone(),
                Tracker {
                    job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                    ..trackers_list[1].clone()
                },
                trackers_list[2].clone(),
                trackers_list[3].clone(),
                trackers_list[4].clone(),
                trackers_list[5].clone()
            ]
        );
        assert_eq!(
            trackers.get_trackers_to_schedule().await?,
            vec![
                trackers_list[0].clone(),
                Tracker {
                    job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                    ..trackers_list[1].clone()
                },
                trackers_list[2].clone()
            ]
        );

        mock_upsert_scheduler_job(
            &db,
            &mock_scheduler_job(
                uuid!("00000000-0000-0000-0000-000000000002"),
                SchedulerJob::TrackersRun,
                "* * * * *",
            ),
        )
        .await?;
        assert_eq!(
            trackers.get_trackers_to_schedule().await?,
            vec![trackers_list[0].clone(), trackers_list[2].clone()]
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_retrieve_tracker_by_job_id(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let trackers_list = [
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .with_schedule("* * * * *")
            .with_job_id(uuid!("00000000-0000-0000-0000-000000000011"))
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000002"),
                "some-name-2",
                3,
            )?
            .with_schedule("* * * * *")
            .with_job_id(uuid!("00000000-0000-0000-0000-000000000022"))
            .build(),
            MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000003"),
                "some-name-3",
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

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
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

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
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
}
