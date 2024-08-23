use crate::trackers::WebPageWaitForState;
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DurationMilliSeconds};
use std::{str::FromStr, time::Duration};
use utoipa::ToSchema;
use void::Void;

/// Describes the web page target option to wait for a specific state of an element before
/// attempting to extract content.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebPageWaitFor {
    /// The CSS selector to locate a target element.
    #[schema(min_length = 1, max_length = 100)]
    pub selector: String,

    /// Optional state of the element to wait for. If not specified, the default state is `Visible`.
    pub state: Option<WebPageWaitForState>,

    /// Optional timeout in milliseconds to wait for the element to reach the desired state. If not
    /// specified, timeout isn't applied.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub timeout: Option<Duration>,
}

impl FromStr for WebPageWaitFor {
    type Err = Void;

    fn from_str(selector: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            selector: selector.to_string(),
            state: None,
            timeout: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WebPageWaitFor;
    use crate::trackers::WebPageWaitForState;
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let wait_for = WebPageWaitFor {
            selector: "div".to_string(),
            state: None,
            timeout: None,
        };
        assert_json_snapshot!(wait_for, @r###"
        {
          "selector": "div"
        }
        "###);

        let wait_for = WebPageWaitFor {
            selector: "div".to_string(),
            state: Some(WebPageWaitForState::Detached),
            timeout: None,
        };
        assert_json_snapshot!(wait_for, @r###"
        {
          "selector": "div",
          "state": "detached"
        }
        "###);

        let wait_for = WebPageWaitFor {
            selector: "div".to_string(),
            state: Some(WebPageWaitForState::Detached),
            timeout: Some(Duration::from_millis(3000)),
        };
        assert_json_snapshot!(wait_for, @r###"
        {
          "selector": "div",
          "state": "detached",
          "timeout": 3000
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let wait_for = WebPageWaitFor {
            selector: "div".to_string(),
            state: None,
            timeout: None,
        };
        assert_eq!(
            serde_json::from_str::<WebPageWaitFor>(&json!({ "selector": "div" }).to_string())?,
            wait_for
        );

        let wait_for = WebPageWaitFor {
            selector: "div".to_string(),
            state: Some(WebPageWaitForState::Detached),
            timeout: None,
        };
        assert_eq!(
            serde_json::from_str::<WebPageWaitFor>(
                &json!({ "selector": "div", "state": "detached" }).to_string()
            )?,
            wait_for
        );

        let wait_for = WebPageWaitFor {
            selector: "div".to_string(),
            state: Some(WebPageWaitForState::Detached),
            timeout: Some(Duration::from_millis(3000)),
        };
        assert_eq!(
            serde_json::from_str::<WebPageWaitFor>(
                &json!({ "selector": "div", "state": "detached", "timeout": 3000 }).to_string()
            )?,
            wait_for
        );

        Ok(())
    }
}
