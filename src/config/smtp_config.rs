use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, DurationMilliSeconds, serde_as};
use std::time::Duration;

/// Configuration for the SMTP functionality.
#[serde_as]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SmtpConfig {
    /// Username to use to authenticate to the SMTP server.
    pub username: String,
    /// Password to use to authenticate to the SMTP server.
    pub password: String,
    /// SMTP server host.
    pub host: String,
    /// SMTP server port. If not specified, default TLS port (465) will be used.
    pub port: Option<u16>,
    /// Whether to NOT use TLS for the SMTP connection.
    #[serde(default)]
    pub no_tls: bool,
    /// Artificial delay between two consecutive emails to avoid hitting SMTP server rate limits.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    #[serde(default = "default_throttle_delay")]
    pub throttle_delay: Duration,
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

/// Default throttle delay between two consecutive emails to avoid hitting SMTP server rate limits.
fn default_throttle_delay() -> Duration {
    Duration::from_secs(5)
}

#[cfg(test)]
mod tests {
    use crate::config::{SmtpConfig, smtp_config::SmtpCatchAllConfig};
    use insta::{assert_debug_snapshot, assert_toml_snapshot};
    use regex::Regex;
    use std::time::Duration;

    #[test]
    fn serialization() {
        let config = SmtpConfig {
            username: "test@retrack.dev".to_string(),
            password: "password".to_string(),
            host: "smtp.retrack.dev".to_string(),
            port: None,
            no_tls: false,
            throttle_delay: Duration::from_secs(10),
            catch_all: None,
        };
        assert_toml_snapshot!(config, @r###"
        username = 'test@retrack.dev'
        password = 'password'
        host = 'smtp.retrack.dev'
        no_tls = false
        throttle_delay = 10000
        "###);

        let config = SmtpConfig {
            username: "test@retrack.dev".to_string(),
            password: "password".to_string(),
            host: "smtp.retrack.dev".to_string(),
            port: Some(465),
            no_tls: true,
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "test@retrack.dev".to_string(),
                text_matcher: Regex::new(r"test").unwrap(),
            }),
            throttle_delay: Duration::from_secs(30),
        };
        assert_toml_snapshot!(config, @r###"
        username = 'test@retrack.dev'
        password = 'password'
        host = 'smtp.retrack.dev'
        port = 465
        no_tls = true
        throttle_delay = 30000

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
        host = 'smtp.retrack.dev'
    "#,
        )
        .unwrap();
        assert_debug_snapshot!(config, @r###"
        SmtpConfig {
            username: "test@retrack.dev",
            password: "password",
            host: "smtp.retrack.dev",
            port: None,
            no_tls: false,
            throttle_delay: 5s,
            catch_all: None,
        }
        "###);

        let config: SmtpConfig = toml::from_str(
            r#"
        username = 'test@retrack.dev'
        password = 'password'
        host = 'smtp.retrack.dev'
        port = 465
        no_tls = true
        throttle_delay = 30000

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
            host: "smtp.retrack.dev",
            port: Some(
                465,
            ),
            no_tls: true,
            throttle_delay: 30s,
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
