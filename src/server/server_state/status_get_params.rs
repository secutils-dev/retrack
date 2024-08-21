use serde::Deserialize;
use utoipa::IntoParams;

/// Parameters for getting a server status.
#[derive(Deserialize, Default, Debug, Copy, Clone, PartialEq, Eq, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct GetStatusParams {
    /// Whether to return an HTTP error code if any of the server component isn't operational.
    #[serde(default)]
    pub fail_if_not_operational: bool,
}

#[cfg(test)]
mod tests {
    use crate::server::GetStatusParams;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<GetStatusParams>(r#"{}"#)?,
            GetStatusParams {
                fail_if_not_operational: false
            }
        );

        assert_eq!(
            serde_json::from_str::<GetStatusParams>(r#"{ "failIfNotOperational": true }"#)?,
            GetStatusParams {
                fail_if_not_operational: true
            }
        );

        Ok(())
    }
}
