use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

/// Tracker's action to record a server log.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerLogAction {
    /// Optional custom script (Deno) to format tracker revision content for action. The script
    /// accepts both previous and current tracker revision content as arguments and should return
    /// a serializable value that will be consumed by the action. If the script is not provided or
    /// returns `null` or `undefined`, the action will receive the current tracker revision content
    /// as is.
    pub formatter: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::ServerLogAction;
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let action = ServerLogAction::default();
        assert_json_snapshot!(action, @"{}");

        let action = ServerLogAction {
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        };
        assert_json_snapshot!(action, @r###"
        {
          "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let action = Default::default();
        assert_eq!(
            serde_json::from_str::<ServerLogAction>(&json!({}).to_string())?,
            action
        );

        let action = ServerLogAction {
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        };
        assert_eq!(
            serde_json::from_str::<ServerLogAction>(
                &json!({ "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();" }).to_string()
            )?,
            action
        );

        Ok(())
    }
}
