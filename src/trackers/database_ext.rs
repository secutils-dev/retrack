mod raw_tracker;
mod raw_tracker_data_revision;
mod raw_tracker_execution_log;

use crate::{
    database::Database,
    error::Error as RetrackError,
    trackers::database_ext::{
        raw_tracker_data_revision::RawTrackerDataRevision,
        raw_tracker_execution_log::RawTrackerExecutionLog,
    },
};
use anyhow::{anyhow, bail};
use raw_tracker::RawTracker;
use retrack_types::trackers::{Tracker, TrackerDataRevision, TrackerExecutionLog};
use sqlx::{Pool, Postgres, error::ErrorKind as SqlxErrorKind, query, query_as};
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

    /// Retrieves all trackers that have all specified tags. If the `tags` field is empty, all
    /// trackers are returned.
    pub async fn get_trackers(&self, tags: &[String]) -> anyhow::Result<Vec<Tracker>> {
        let raw_trackers = if tags.is_empty() {
            query_as!(
                RawTracker,
                r#"
SELECT t.id, t.name, t.enabled, t.config, t.tags, t.created_at, t.updated_at,
       t.job_id, t.job_needed,
       to_timestamp(sj.next_tick) as scheduled_at,
       to_timestamp(sj.last_tick) as last_ran_at
FROM trackers t
LEFT JOIN scheduler_jobs sj ON t.job_id = sj.id
ORDER BY t.updated_at, t.id
                "#
            )
            .fetch_all(self.pool)
            .await?
        } else {
            query_as!(
                RawTracker,
                r#"
SELECT t.id, t.name, t.enabled, t.config, t.tags, t.created_at, t.updated_at,
       t.job_id, t.job_needed,
       to_timestamp(sj.next_tick) as scheduled_at,
       to_timestamp(sj.last_tick) as last_ran_at
FROM trackers t
LEFT JOIN scheduler_jobs sj ON t.job_id = sj.id
WHERE t.tags @> $1
ORDER BY t.updated_at, t.id
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
SELECT t.id, t.name, t.enabled, t.config, t.tags, t.created_at, t.updated_at,
       t.job_id, t.job_needed,
       to_timestamp(sj.next_tick) as scheduled_at,
       to_timestamp(sj.last_tick) as last_ran_at
FROM trackers t
LEFT JOIN scheduler_jobs sj ON t.job_id = sj.id
WHERE t.id = $1
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
SELECT t.id, t.name, t.enabled, t.config, t.tags, t.created_at, t.updated_at,
       t.job_needed, t.job_id,
       to_timestamp(sj.next_tick) as scheduled_at,
       to_timestamp(sj.last_tick) as last_ran_at
FROM trackers as t
LEFT JOIN scheduler_jobs sj ON t.job_id = sj.id
WHERE t.job_needed = TRUE AND t.enabled = TRUE AND (t.job_id IS NULL OR sj.id IS NULL)
ORDER BY t.updated_at, t.id
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
SELECT t.id, t.name, t.enabled, t.config, t.tags, t.created_at, t.updated_at,
       t.job_needed, t.job_id,
       to_timestamp(sj.next_tick) as scheduled_at,
       to_timestamp(sj.last_tick) as last_ran_at
FROM trackers t
LEFT JOIN scheduler_jobs sj ON t.job_id = sj.id
WHERE t.job_id = $1
            "#,
            job_id
        )
        .fetch_optional(self.pool)
        .await?
        .map(Tracker::try_from)
        .transpose()
    }

    /// Inserts a tracker execution log entry.
    pub async fn insert_tracker_execution_log(
        &self,
        log: &TrackerExecutionLog,
    ) -> anyhow::Result<()> {
        let raw = RawTrackerExecutionLog::try_from(log)?;
        query!(
            r#"
    INSERT INTO tracker_execution_logs (id, tracker_id, job_id, started_at, finished_at, status,
                                        error, is_manual, retry_attempt, max_retry_attempts,
                                        revision_size, has_changes, duration_ms, phases)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            "#,
            raw.id,
            raw.tracker_id,
            raw.job_id,
            raw.started_at,
            raw.finished_at,
            raw.status,
            raw.error,
            raw.is_manual,
            raw.retry_attempt,
            raw.max_retry_attempts,
            raw.revision_size,
            raw.has_changes,
            raw.duration_ms,
            raw.phases,
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Retrieves execution logs for a tracker, ordered by start time descending.
    pub async fn get_tracker_execution_logs(
        &self,
        tracker_id: Uuid,
        size: usize,
    ) -> anyhow::Result<Vec<TrackerExecutionLog>> {
        let raw_logs = query_as!(
            RawTrackerExecutionLog,
            r#"
SELECT id, tracker_id, job_id, started_at, finished_at, status, error, is_manual,
       retry_attempt, max_retry_attempts, revision_size, has_changes, duration_ms, phases
FROM tracker_execution_logs
WHERE tracker_id = $1
ORDER BY started_at DESC
LIMIT $2
            "#,
            tracker_id,
            size as i64
        )
        .fetch_all(self.pool)
        .await?;

        raw_logs
            .into_iter()
            .map(TrackerExecutionLog::try_from)
            .collect()
    }

    /// Retrieves execution logs for multiple trackers in a single query, returning up to `size`
    /// entries per tracker ordered by start time descending.
    pub async fn get_tracker_execution_logs_batch(
        &self,
        tracker_ids: &[Uuid],
        size: usize,
    ) -> anyhow::Result<Vec<TrackerExecutionLog>> {
        if tracker_ids.is_empty() {
            return Ok(vec![]);
        }

        let raw_logs = query_as!(
            RawTrackerExecutionLog,
            r#"
SELECT l.id, l.tracker_id, l.job_id, l.started_at, l.finished_at, l.status, l.error,
       l.is_manual, l.retry_attempt, l.max_retry_attempts, l.revision_size, l.has_changes, l.duration_ms, l.phases
FROM unnest($1::uuid[]) AS t(tracker_id)
CROSS JOIN LATERAL (
    SELECT *
    FROM tracker_execution_logs el
    WHERE el.tracker_id = t.tracker_id
    ORDER BY el.started_at DESC
    LIMIT $2
) l
            "#,
            tracker_ids,
            size as i64
        )
        .fetch_all(self.pool)
        .await?;

        raw_logs
            .into_iter()
            .map(TrackerExecutionLog::try_from)
            .collect()
    }

    /// Removes all execution logs for a specific tracker.
    pub async fn clear_tracker_execution_logs(&self, tracker_id: Uuid) -> anyhow::Result<()> {
        query!(
            r#"DELETE FROM tracker_execution_logs WHERE tracker_id = $1"#,
            tracker_id
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Removes all execution logs for all trackers.
    pub async fn clear_all_tracker_execution_logs(&self) -> anyhow::Result<()> {
        query!(r#"DELETE FROM tracker_execution_logs"#)
            .execute(self.pool)
            .await?;

        Ok(())
    }

    /// Removes execution logs older than the specified cutoff time.
    pub async fn cleanup_tracker_execution_logs(
        &self,
        cutoff: OffsetDateTime,
    ) -> anyhow::Result<u64> {
        let result = query!(
            r#"DELETE FROM tracker_execution_logs WHERE started_at < $1"#,
            cutoff
        )
        .execute(self.pool)
        .await?;

        Ok(result.rows_affected())
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
    use retrack_types::trackers::{
        Tracker, TrackerDataRevision, TrackerDataValue, TrackerExecutionLog,
        TrackerExecutionLogPhase, TrackerExecutionLogStatus,
    };
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

    fn create_mock_execution_log(
        id: Uuid,
        tracker_id: Uuid,
        time_shift: i64,
    ) -> anyhow::Result<TrackerExecutionLog> {
        Ok(TrackerExecutionLog {
            id,
            tracker_id,
            job_id: None,
            started_at: OffsetDateTime::from_unix_timestamp(946720800 + time_shift)?,
            finished_at: OffsetDateTime::from_unix_timestamp(946720803 + time_shift)?,
            status: TrackerExecutionLogStatus::Success,
            error: None,
            is_manual: true,
            retry_attempt: None,
            max_retry_attempts: None,
            revision_size: None,
            has_changes: None,
            duration_ms: 3000,
            phases: None,
        })
    }

    #[sqlx::test]
    async fn can_insert_and_retrieve_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test", 3)?
                .build();
        db.trackers().insert_tracker(&tracker).await?;

        let log = TrackerExecutionLog {
            id: uuid!("00000000-0000-0000-0000-000000000010"),
            tracker_id: tracker.id,
            job_id: Some(uuid!("00000000-0000-0000-0000-000000000099")),
            started_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            finished_at: OffsetDateTime::from_unix_timestamp(946720803)?,
            status: TrackerExecutionLogStatus::Success,
            error: None,
            is_manual: false,
            retry_attempt: Some(0),
            max_retry_attempts: Some(3),
            revision_size: Some(4521),
            has_changes: Some(true),
            duration_ms: 2340,
            phases: Some(vec![TrackerExecutionLogPhase {
                phase: "fetch_data".to_string(),
                duration_ms: 2340,
                status: TrackerExecutionLogStatus::Success,
                meta: Some(json!({"statusCode": 200})),
            }]),
        };
        db.trackers().insert_tracker_execution_log(&log).await?;

        let logs = db
            .trackers()
            .get_tracker_execution_logs(tracker.id, 50)
            .await?;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0], log);

        Ok(())
    }

    #[sqlx::test]
    async fn retrieves_execution_logs_in_desc_order(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test", 3)?
                .build();
        db.trackers().insert_tracker(&tracker).await?;

        let log_older = create_mock_execution_log(
            uuid!("00000000-0000-0000-0000-000000000010"),
            tracker.id,
            0,
        )?;
        let log_newer = create_mock_execution_log(
            uuid!("00000000-0000-0000-0000-000000000011"),
            tracker.id,
            100,
        )?;

        db.trackers()
            .insert_tracker_execution_log(&log_older)
            .await?;
        db.trackers()
            .insert_tracker_execution_log(&log_newer)
            .await?;

        let logs = db
            .trackers()
            .get_tracker_execution_logs(tracker.id, 50)
            .await?;
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].id, log_newer.id);
        assert_eq!(logs[1].id, log_older.id);

        Ok(())
    }

    #[sqlx::test]
    async fn respects_size_limit_for_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test", 3)?
                .build();
        db.trackers().insert_tracker(&tracker).await?;

        for i in 0..5 {
            let log =
                create_mock_execution_log(Uuid::from_u128(0x10 + i), tracker.id, i as i64 * 10)?;
            db.trackers().insert_tracker_execution_log(&log).await?;
        }

        let logs = db
            .trackers()
            .get_tracker_execution_logs(tracker.id, 2)
            .await?;
        assert_eq!(logs.len(), 2);

        Ok(())
    }

    #[sqlx::test]
    async fn can_clear_execution_logs_for_tracker(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker1 =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test-1", 3)?
                .build();
        let tracker2 =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000002"), "test-2", 3)?
                .build();
        db.trackers().insert_tracker(&tracker1).await?;
        db.trackers().insert_tracker(&tracker2).await?;

        let log1 = create_mock_execution_log(
            uuid!("00000000-0000-0000-0000-000000000010"),
            tracker1.id,
            0,
        )?;
        let log2 = create_mock_execution_log(
            uuid!("00000000-0000-0000-0000-000000000020"),
            tracker2.id,
            10,
        )?;
        db.trackers().insert_tracker_execution_log(&log1).await?;
        db.trackers().insert_tracker_execution_log(&log2).await?;

        db.trackers()
            .clear_tracker_execution_logs(tracker1.id)
            .await?;

        let logs1 = db
            .trackers()
            .get_tracker_execution_logs(tracker1.id, 50)
            .await?;
        let logs2 = db
            .trackers()
            .get_tracker_execution_logs(tracker2.id, 50)
            .await?;
        assert!(logs1.is_empty());
        assert_eq!(logs2.len(), 1);

        Ok(())
    }

    #[sqlx::test]
    async fn can_clear_all_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker1 =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test-1", 3)?
                .build();
        let tracker2 =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000002"), "test-2", 3)?
                .build();
        db.trackers().insert_tracker(&tracker1).await?;
        db.trackers().insert_tracker(&tracker2).await?;

        for (i, tid) in [tracker1.id, tracker2.id].iter().enumerate() {
            let log =
                create_mock_execution_log(Uuid::from_u128(0x10 + i as u128), *tid, i as i64 * 10)?;
            db.trackers().insert_tracker_execution_log(&log).await?;
        }

        db.trackers().clear_all_tracker_execution_logs().await?;

        let logs1 = db
            .trackers()
            .get_tracker_execution_logs(tracker1.id, 50)
            .await?;
        let logs2 = db
            .trackers()
            .get_tracker_execution_logs(tracker2.id, 50)
            .await?;
        assert!(logs1.is_empty());
        assert!(logs2.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_cleanup_old_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test", 3)?
                .build();
        db.trackers().insert_tracker(&tracker).await?;

        // Old log (before cutoff).
        let old_log = create_mock_execution_log(
            uuid!("00000000-0000-0000-0000-000000000010"),
            tracker.id,
            0,
        )?;
        // Recent log (after cutoff).
        let new_log = create_mock_execution_log(
            uuid!("00000000-0000-0000-0000-000000000011"),
            tracker.id,
            200,
        )?;
        db.trackers().insert_tracker_execution_log(&old_log).await?;
        db.trackers().insert_tracker_execution_log(&new_log).await?;

        let cutoff = OffsetDateTime::from_unix_timestamp(946720800 + 100)?;
        let deleted = db.trackers().cleanup_tracker_execution_logs(cutoff).await?;
        assert_eq!(deleted, 1);

        let logs = db
            .trackers()
            .get_tracker_execution_logs(tracker.id, 50)
            .await?;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].id, new_log.id);

        Ok(())
    }

    #[sqlx::test]
    async fn execution_logs_deleted_on_tracker_delete(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test", 3)?
                .build();
        db.trackers().insert_tracker(&tracker).await?;

        let log = create_mock_execution_log(
            uuid!("00000000-0000-0000-0000-000000000010"),
            tracker.id,
            0,
        )?;
        db.trackers().insert_tracker_execution_log(&log).await?;

        assert_eq!(
            db.trackers()
                .get_tracker_execution_logs(tracker.id, 50)
                .await?
                .len(),
            1
        );

        db.trackers().remove_tracker(tracker.id).await?;

        let logs = db
            .trackers()
            .get_tracker_execution_logs(tracker.id, 50)
            .await?;
        assert!(logs.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_insert_execution_log_with_failure_status(pool: PgPool) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;
        let tracker =
            MockTrackerBuilder::create(uuid!("00000000-0000-0000-0000-000000000001"), "test", 3)?
                .build();
        db.trackers().insert_tracker(&tracker).await?;

        let log = TrackerExecutionLog {
            id: uuid!("00000000-0000-0000-0000-000000000010"),
            tracker_id: tracker.id,
            job_id: None,
            started_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            finished_at: OffsetDateTime::from_unix_timestamp(946720801)?,
            status: TrackerExecutionLogStatus::Failure,
            error: Some("Connection timeout".to_string()),
            is_manual: true,
            retry_attempt: Some(2),
            max_retry_attempts: Some(3),
            revision_size: None,
            has_changes: None,
            duration_ms: 1000,
            phases: None,
        };
        db.trackers().insert_tracker_execution_log(&log).await?;

        let logs = db
            .trackers()
            .get_tracker_execution_logs(tracker.id, 50)
            .await?;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0], log);

        Ok(())
    }

    #[sqlx::test]
    async fn get_tracker_returns_schedule_timestamps_from_scheduler_job(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "scheduled",
            3,
        )?
        .with_schedule("0 0 * * *")
        .with_job_id(uuid!("00000000-0000-0000-0000-000000000011"))
        .build();
        db.trackers().insert_tracker(&tracker).await?;

        // Before inserting the scheduler job, both fields should be None.
        let fetched = db.trackers().get_tracker(tracker.id).await?.unwrap();
        assert_eq!(fetched.scheduled_at, None);
        assert_eq!(fetched.last_ran_at, None);

        // Insert a scheduler job with next_tick and last_tick.
        let mut job = mock_scheduler_job(
            uuid!("00000000-0000-0000-0000-000000000011"),
            SchedulerJob::TrackersRun,
            "0 0 * * *",
        );
        job.next_tick = Some(946720900);
        job.last_tick = Some(946720700);
        mock_upsert_scheduler_job(&db, &job).await?;

        let fetched = db.trackers().get_tracker(tracker.id).await?.unwrap();
        let expected = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "scheduled",
            3,
        )?
        .with_schedule("0 0 * * *")
        .with_job_id(uuid!("00000000-0000-0000-0000-000000000011"))
        .with_scheduled_at(OffsetDateTime::from_unix_timestamp(946720900)?)
        .with_last_ran_at(OffsetDateTime::from_unix_timestamp(946720700)?)
        .build();
        assert_eq!(fetched, expected);

        Ok(())
    }

    #[sqlx::test]
    async fn get_tracker_returns_none_schedule_timestamps_without_job(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "no-schedule",
            3,
        )?
        .build();
        db.trackers().insert_tracker(&tracker).await?;

        let fetched = db.trackers().get_tracker(tracker.id).await?.unwrap();
        assert_eq!(fetched.scheduled_at, None);
        assert_eq!(fetched.last_ran_at, None);

        Ok(())
    }

    #[sqlx::test]
    async fn get_trackers_returns_correct_schedule_timestamps_for_mixed_trackers(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let db = Database::create(pool).await?;

        let tracker_with_job = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "with-job",
            3,
        )?
        .with_schedule("0 0 * * *")
        .with_job_id(uuid!("00000000-0000-0000-0000-000000000011"))
        .build();

        let tracker_without_job = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000002"),
            "without-job",
            3,
        )?
        .build();

        let trackers = db.trackers();
        trackers.insert_tracker(&tracker_with_job).await?;
        trackers.insert_tracker(&tracker_without_job).await?;

        let mut job = mock_scheduler_job(
            uuid!("00000000-0000-0000-0000-000000000011"),
            SchedulerJob::TrackersRun,
            "0 0 * * *",
        );
        job.next_tick = Some(946720900);
        job.last_tick = Some(946720700);
        mock_upsert_scheduler_job(&db, &job).await?;

        let all_trackers = trackers.get_trackers(&[]).await?;
        assert_eq!(all_trackers.len(), 2);

        let fetched_with_job = all_trackers
            .iter()
            .find(|t| t.id == tracker_with_job.id)
            .unwrap();
        assert_eq!(
            fetched_with_job.scheduled_at,
            Some(OffsetDateTime::from_unix_timestamp(946720900)?)
        );
        assert_eq!(
            fetched_with_job.last_ran_at,
            Some(OffsetDateTime::from_unix_timestamp(946720700)?)
        );

        let fetched_without_job = all_trackers
            .iter()
            .find(|t| t.id == tracker_without_job.id)
            .unwrap();
        assert_eq!(fetched_without_job.scheduled_at, None);
        assert_eq!(fetched_without_job.last_ran_at, None);

        Ok(())
    }
}
