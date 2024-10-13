use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's action to send an email.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailAction {
    /// An email will be sent to the specified emails.
    pub to: Vec<String>,
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
        };
        assert_json_snapshot!(action, @r###"
        {
          "to": [
            "dev@retrack.dev",
            "dev-2@retrack.dev"
          ]
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
        };
        assert_eq!(
            serde_json::from_str::<EmailAction>(
                &json!({ "to": [ "dev@retrack.dev", "dev-2@retrack.dev" ] }).to_string()
            )?,
            action
        );

        Ok(())
    }
}
