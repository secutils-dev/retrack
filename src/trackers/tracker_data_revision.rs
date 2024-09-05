use serde::Serialize;
use serde_json::Value as JsonValue;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// Represents a tracker data revision.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDataRevision {
    /// Unique tracker data revision id (UUIDv7).
    pub id: Uuid,
    /// ID of the tracker captured data belongs to.
    #[serde(skip_serializing)]
    pub tracker_id: Uuid,
    /// Tracker data revision value.
    pub data: JsonValue,
    /// Timestamp indicating when data was fetched.
    #[serde(with = "time::serde::timestamp")]
    pub created_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use crate::trackers::TrackerDataRevision;
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
            data: json!("some-data"),
        }, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "data": "some-data",
          "createdAt": 946720800
        }
        "###);

        Ok(())
    }
}
