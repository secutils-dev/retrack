use crate::trackers::TrackerDataValue;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// Represents a tracker data revision.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDataRevision<TValue = serde_json::Value> {
    /// Unique tracker data revision id (UUIDv7).
    pub id: Uuid,
    /// ID of the tracker captured data belongs to.
    pub tracker_id: Uuid,
    /// Array of tracker data revision values including the original one and the those potentially
    /// transformed by the tracker actions, if any.
    pub data: TrackerDataValue<TValue>,
    /// Timestamp indicating when data was fetched.
    #[serde(with = "time::serde::timestamp")]
    pub created_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{TrackerDataRevision, TrackerDataValue};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            created_at: OffsetDateTime::from_unix_timestamp(
                946720800,
            )?,
            data: TrackerDataValue::new(json!("some-data")),
        }, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "trackerId": "00000000-0000-0000-0000-000000000002",
          "data": {
            "original": "some-data"
          },
          "createdAt": 946720800
        }
        "###);

        Ok(())
    }
}
