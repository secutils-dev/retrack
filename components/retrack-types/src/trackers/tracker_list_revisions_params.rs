use serde::Deserialize;
use utoipa::IntoParams;

/// Parameters for getting a list of revisions of a tracker.
#[derive(Deserialize, Default, Debug, Copy, Clone, PartialEq, Eq, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct TrackerListRevisionsParams {
    /// Whether to calculate the diff between the returned data revisions.
    #[serde(default)]
    pub calculate_diff: bool,
}

#[cfg(test)]
mod tests {
    use crate::trackers::TrackerListRevisionsParams;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerListRevisionsParams>(r#"{}"#)?,
            TrackerListRevisionsParams {
                calculate_diff: false
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerListRevisionsParams>(
                r#"
{
    "calculateDiff": true
}
          "#
            )?,
            TrackerListRevisionsParams {
                calculate_diff: true
            }
        );

        Ok(())
    }
}
