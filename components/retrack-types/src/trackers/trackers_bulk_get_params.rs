use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Parameters for bulk-fetching trackers by their IDs.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackersBulkGetParams {
    /// A list of tracker IDs to retrieve.
    pub ids: Vec<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::TrackersBulkGetParams;
    use uuid::uuid;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackersBulkGetParams>(r#"{"ids": []}"#)?,
            TrackersBulkGetParams { ids: vec![] }
        );

        assert_eq!(
            serde_json::from_str::<TrackersBulkGetParams>(
                r#"{"ids": ["00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002"]}"#
            )?,
            TrackersBulkGetParams {
                ids: vec![
                    uuid!("00000000-0000-0000-0000-000000000001"),
                    uuid!("00000000-0000-0000-0000-000000000002"),
                ],
            }
        );

        Ok(())
    }
}
