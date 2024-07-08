use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, DurationMilliSeconds};
use std::time::Duration;
use utoipa::ToSchema;

/// Tracker's target for a web page.
#[serde_as]
#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerWebPageTarget {
    /// Number of milliseconds to wait after web page enters "idle" state to start tracking.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub delay: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::TrackerWebPageTarget;
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = TrackerWebPageTarget::default();
        assert_json_snapshot!(target, @r###"
        {
          "delay": null
        }
        "###);

        let target = TrackerWebPageTarget {
            delay: Some(Duration::from_millis(2500)),
        };
        assert_json_snapshot!(target, @r###"
        {
          "delay": 2500
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = TrackerWebPageTarget::default();
        assert_eq!(
            serde_json::from_str::<TrackerWebPageTarget>(&json!({}).to_string())?,
            target
        );

        let target = TrackerWebPageTarget {
            delay: Some(Duration::from_millis(2000)),
        };
        assert_eq!(
            serde_json::from_str::<TrackerWebPageTarget>(&json!({ "delay": 2000 }).to_string())?,
            target
        );

        Ok(())
    }
}
