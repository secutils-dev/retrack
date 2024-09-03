use serde_derive::{Deserialize, Serialize};
use url::Url;
use utoipa::ToSchema;

/// Tracker's target for JSON API.
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JsonApiTarget {
    /// URL of the API endpoint that returns JSON to track.
    pub url: Url,
}

#[cfg(test)]
mod tests {
    use crate::trackers::JsonApiTarget;
    use insta::assert_json_snapshot;
    use serde_json::json;
    use url::Url;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = JsonApiTarget {
            url: Url::parse("https://retrack.dev")?,
        };
        assert_json_snapshot!(target, @r###"
        {
          "url": "https://retrack.dev/"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = JsonApiTarget {
            url: Url::parse("https://retrack.dev")?,
        };
        assert_eq!(
            serde_json::from_str::<JsonApiTarget>(
                &json!({ "url": "https://retrack.dev" }).to_string()
            )?,
            target
        );

        Ok(())
    }
}
