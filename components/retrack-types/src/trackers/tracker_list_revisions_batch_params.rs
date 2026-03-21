use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Default maximum number of revision entries per tracker for batch queries.
pub const DEFAULT_REVISIONS_BATCH_SIZE: usize = 10;

/// Parameters for getting a batch of revisions for multiple trackers.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerListRevisionsBatchParams {
    /// A list of tracker IDs to retrieve revisions for.
    pub tracker_ids: Vec<Uuid>,
    /// The maximum number of revision entries to return per tracker. Defaults to 10.
    #[serde(default)]
    pub size: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::TrackerListRevisionsBatchParams;
    use uuid::uuid;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerListRevisionsBatchParams>(
                r#"{"trackerIds": ["00000000-0000-0000-0000-000000000001"]}"#
            )?,
            TrackerListRevisionsBatchParams {
                tracker_ids: vec![uuid!("00000000-0000-0000-0000-000000000001")],
                size: None,
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerListRevisionsBatchParams>(
                r#"{"trackerIds": ["00000000-0000-0000-0000-000000000001"], "size": 5}"#
            )?,
            TrackerListRevisionsBatchParams {
                tracker_ids: vec![uuid!("00000000-0000-0000-0000-000000000001")],
                size: Some(5),
            }
        );

        Ok(())
    }
}
