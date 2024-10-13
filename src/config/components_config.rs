use serde::{Deserialize, Serialize};
use url::Url;

/// Configuration for the main Retrack components.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct ComponentsConfig {
    /// The URL to access the Web Scraper component.
    pub web_scraper_url: Url,
}

impl Default for ComponentsConfig {
    fn default() -> Self {
        Self {
            web_scraper_url: Url::parse("http://localhost:7272")
                .expect("Cannot parse Web Scraper URL parameter."),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ComponentsConfig;
    use insta::assert_toml_snapshot;

    #[test]
    fn serialization_and_default() {
        assert_toml_snapshot!(ComponentsConfig::default(), @r###"
        web_scraper_url = 'http://localhost:7272/'
        "###);
    }

    #[test]
    fn deserialization() {
        let config: ComponentsConfig = toml::from_str(
            r#"
        web_scraper_url = 'http://localhost:7272/'
    "#,
        )
        .unwrap();
        assert_eq!(config, ComponentsConfig::default());
    }
}
