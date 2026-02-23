use serde::Deserialize;
use std::num::NonZero;
use utoipa::IntoParams;

/// Default number of context lines to include around each changed hunk in the diff output.
pub const DEFAULT_DIFF_CONTEXT_RADIUS: usize = 3;

/// Maximum allowed value for the diff context radius.
pub const MAX_DIFF_CONTEXT_RADIUS: usize = 10000;

/// Parameters for getting a list of revisions of a tracker.
#[derive(Deserialize, Default, Debug, Copy, Clone, PartialEq, Eq, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct TrackerListRevisionsParams {
    /// Whether to calculate the diff between the returned data revisions.
    #[serde(default)]
    pub calculate_diff: bool,
    /// The number of unchanged context lines to include around each changed hunk in the diff. Only
    /// used when `calculate_diff` is true. Defaults to 3, max 10000.
    #[serde(default)]
    #[param(value_type = usize, minimum = 0, maximum = 10000)]
    pub context_radius: Option<usize>,
    /// The number of data revisions to return. The value should be a positive, non-zero number. If
    /// not set, all revisions are returned. Data revisions are sorted by creation date, with the
    /// newest revision returned first.
    #[serde(default)]
    #[param(value_type = usize, minimum = 1)]
    pub size: Option<NonZero<usize>>,
}

impl TrackerListRevisionsParams {
    /// Returns the effective context radius, clamped to the allowed maximum.
    pub fn effective_context_radius(&self) -> usize {
        self.context_radius
            .unwrap_or(DEFAULT_DIFF_CONTEXT_RADIUS)
            .min(MAX_DIFF_CONTEXT_RADIUS)
    }
}

#[cfg(test)]
mod tests {
    use crate::trackers::{
        TrackerListRevisionsParams,
        tracker_list_revisions_params::{DEFAULT_DIFF_CONTEXT_RADIUS, MAX_DIFF_CONTEXT_RADIUS},
    };

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
                context_radius: None,
                size: Some(10.try_into()?)
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerListRevisionsParams>(
                r#"
{
    "calculateDiff": true,
    "contextRadius": 5,
    "size": 10
}
          "#
            )?,
            TrackerListRevisionsParams {
                calculate_diff: true,
                context_radius: Some(5),
                size: Some(10.try_into()?)
            }
        );

        Ok(())
    }

    #[test]
    fn effective_context_radius() {
        let params = TrackerListRevisionsParams::default();
        assert_eq!(
            params.effective_context_radius(),
            DEFAULT_DIFF_CONTEXT_RADIUS
        );

        let params = TrackerListRevisionsParams {
            context_radius: Some(7),
            ..Default::default()
        };
        assert_eq!(params.effective_context_radius(), 7);

        let params = TrackerListRevisionsParams {
            context_radius: Some(MAX_DIFF_CONTEXT_RADIUS + 100),
            ..Default::default()
        };
        assert_eq!(params.effective_context_radius(), MAX_DIFF_CONTEXT_RADIUS);

        let params = TrackerListRevisionsParams {
            context_radius: Some(0),
            ..Default::default()
        };
        assert_eq!(params.effective_context_radius(), 0);
    }
}
