use serde::Deserialize;
use utoipa::IntoParams;

/// Parameters for getting a list of revisions of a tracker.
#[derive(Deserialize, Default, Debug, Clone, PartialEq, Eq, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct TrackersListParams {
    /// List of tags to filter trackers by.
    #[param(max_items = 10, min_length = 1, max_length = 50)]
    #[serde(default, rename = "tag")]
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::TrackersListParams;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackersListParams>(r#"{}"#)?,
            TrackersListParams { tags: vec![] }
        );

        assert_eq!(
            serde_json::from_str::<TrackersListParams>(
                r#"
{
    "tag": ["tag_one", "tag_two"]
}
          "#
            )?,
            TrackersListParams {
                tags: vec!["tag_one".to_string(), "tag_two".to_string()]
            }
        );

        Ok(())
    }
}
