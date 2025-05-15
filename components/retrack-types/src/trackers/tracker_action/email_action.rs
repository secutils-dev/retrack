use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

/// Tracker's action to send an email.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailAction {
    /// An email will be sent to the specified emails.
    pub to: Vec<String>,
    /// Optional custom script (Deno) to format tracker revision content for action. The script
    /// accepts both previous and current tracker revision content as arguments and should return
    /// a serializable value that will be consumed by the action. If the script is not provided or
    /// returns `null` or `undefined`, the action will receive the current tracker revision content
    /// as is.
    pub formatter: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::EmailAction;
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let action = EmailAction {
            to: vec![
                "dev@retrack.dev".to_string(),
                "dev-2@retrack.dev".to_string(),
            ],
            formatter: None,
        };
        assert_json_snapshot!(action, @r###"
        {
          "to": [
            "dev@retrack.dev",
            "dev-2@retrack.dev"
          ]
        }
        "###);

        let action = EmailAction {
            to: vec![
                "dev@retrack.dev".to_string(),
                "dev-2@retrack.dev".to_string(),
            ],
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        };
        assert_json_snapshot!(action, @r###"
        {
          "to": [
            "dev@retrack.dev",
            "dev-2@retrack.dev"
          ],
          "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let action = EmailAction {
            to: vec![
                "dev@retrack.dev".to_string(),
                "dev-2@retrack.dev".to_string(),
            ],
            formatter: None,
        };
        assert_eq!(
            serde_json::from_str::<EmailAction>(
                &json!({ "to": [ "dev@retrack.dev", "dev-2@retrack.dev" ] }).to_string()
            )?,
            action
        );

        let action = EmailAction {
            to: vec![
                "dev@retrack.dev".to_string(),
                "dev-2@retrack.dev".to_string(),
            ],
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        };
        assert_eq!(
            serde_json::from_str::<EmailAction>(
                &json!({
                    "to": ["dev@retrack.dev", "dev-2@retrack.dev"],
                    "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                }).to_string()
            )?,
            action
        );

        Ok(())
    }
}
