use serde::Deserialize;
use std::num::NonZero;
use utoipa::IntoParams;

/// Parameters for getting a list of revisions of a tracker.
#[derive(Deserialize, Default, Debug, Copy, Clone, PartialEq, Eq, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct TrackerListRevisionsParams {
    /// Whether to calculate the diff between the returned data revisions.
    #[serde(default)]
    pub calculate_diff: bool,
    /// The number of data revisions to return. The value should be a positive, non-zero number. If
    /// not set, all revisions are returned. Data revisions are sorted by creation date, with the
    /// newest revision returned first.
    #[serde(default)]
    #[param(value_type = usize, minimum = 1)]
    pub size: Option<NonZero<usize>>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::TrackerListRevisionsParams;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerListRevisionsParams>(r#"{}"#)?,
            Default::default()
        );

        assert_eq!(
            serde_json::from_str::<TrackerListRevisionsParams>(
                r#"
{
    "calculateDiff": true,
    "size": 10
}
          "#
            )?,
            TrackerListRevisionsParams {
                calculate_diff: true,
                size: Some(10.try_into()?)
            }
        );

        Ok(())
    }
}
