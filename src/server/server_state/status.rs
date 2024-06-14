use serde::Serialize;
use utoipa::ToSchema;

/// Server status.
#[derive(Clone, Serialize, ToSchema)]
pub struct Status {
    /// Version of the server.
    pub version: String,
}

#[cfg(test)]
mod tests {
    use crate::server::Status;
    use insta::assert_json_snapshot;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(Status {
            version: "1.0.0-alpha.4".to_string()
        }, @r###"
        {
          "version": "1.0.0-alpha.4"
        }
        "###);

        Ok(())
    }
}
