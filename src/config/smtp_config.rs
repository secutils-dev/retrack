use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};

/// Configuration for the SMTP functionality.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SmtpConfig {
    /// Username to use to authenticate to the SMTP server.
    pub username: String,
    /// Password to use to authenticate to the SMTP server.
    pub password: String,
    /// Address of the SMTP server.
    pub address: String,
    /// Optional configuration for catch-all email recipient (used for troubleshooting only).
    pub catch_all: Option<SmtpCatchAllConfig>,
}

/// Configuration for the SMTP catch-all functionality.
#[serde_as]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SmtpCatchAllConfig {
    /// Address of the catch-all email recipient.
    pub recipient: String,
    /// Email will be sent to the catch-all recipient instead of original one only if the email text
    /// matches regular expression specified in `text_matcher`.
    #[serde_as(as = "DisplayFromStr")]
    pub text_matcher: Regex,
}

#[cfg(test)]
mod tests {
    use crate::config::{smtp_config::SmtpCatchAllConfig, SmtpConfig};
    use insta::{assert_debug_snapshot, assert_toml_snapshot};
    use regex::Regex;

    #[test]
    fn serialization() {
        let config = SmtpConfig {
            username: "test@retrack.dev".to_string(),
            password: "password".to_string(),
            address: "smtp.retrack.dev".to_string(),
            catch_all: None,
        };
        assert_toml_snapshot!(config, @r###"
        username = 'test@retrack.dev'
        password = 'password'
        address = 'smtp.retrack.dev'
        "###);

        let config = SmtpConfig {
            username: "test@retrack.dev".to_string(),
            password: "password".to_string(),
            address: "smtp.retrack.dev".to_string(),
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "test@retrack.dev".to_string(),
                text_matcher: Regex::new(r"test").unwrap(),
            }),
        };
        assert_toml_snapshot!(config, @r###"
        username = 'test@retrack.dev'
        password = 'password'
        address = 'smtp.retrack.dev'

        [catch_all]
        recipient = 'test@retrack.dev'
        text_matcher = 'test'
        "###);
    }

    #[test]
    fn deserialization() {
        let config: SmtpConfig = toml::from_str(
            r#"
        username = 'test@retrack.dev'
        password = 'password'
        address = 'smtp.retrack.dev'

        [catch_all]
        recipient = 'test@retrack.dev'
        text_matcher = 'test'
    "#,
        )
        .unwrap();
        assert_debug_snapshot!(config, @r###"
        SmtpConfig {
            username: "test@retrack.dev",
            password: "password",
            address: "smtp.retrack.dev",
            catch_all: Some(
                SmtpCatchAllConfig {
                    recipient: "test@retrack.dev",
                    text_matcher: Regex(
                        "test",
                    ),
                },
            ),
        }
        "###);
    }
}
