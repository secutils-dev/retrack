use crate::trackers::TrackerDataValue;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

/// Parameters for importing a single tracker data revision.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDataRevisionImportParams {
    /// Array of tracker data revision values including the original one and the those potentially
    /// transformed by the tracker actions, if any.
    pub data: TrackerDataValue,
    /// Timestamp indicating when data was fetched.
    #[serde(with = "time::serde::timestamp")]
    pub created_at: OffsetDateTime,
}

/// Result of a bulk revision import operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDataRevisionImportResult {
    /// Number of revisions successfully imported.
    pub imported: usize,
    /// Number of revisions skipped (e.g., due to duplicate timestamps).
    pub skipped: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trackers::TrackerDataValue;
    use insta::assert_json_snapshot;
    use serde_json::json;
    use time::OffsetDateTime;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let params = TrackerDataRevisionImportParams {
            data: TrackerDataValue::new(json!("some-data")),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        };
        assert_json_snapshot!(params, @r###"
        {
          "data": {
            "original": "some-data"
          },
          "createdAt": 946720800
        }
        "###);

        Ok(())
    }

    #[test]
    fn result_serialization() -> anyhow::Result<()> {
        let result = TrackerDataRevisionImportResult {
            imported: 5,
            skipped: 2,
        };
        assert_json_snapshot!(result, @r###"
        {
          "imported": 5,
          "skipped": 2
        }
        "###);

        Ok(())
    }
}
