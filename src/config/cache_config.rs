use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for the various caches Retrack relies on.
#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq)]
pub struct CacheConfig {
    /// The directory where the HTTP cache will be stored.
    pub http_cache_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use crate::config::cache_config::CacheConfig;
    use insta::assert_toml_snapshot;

    #[test]
    fn serialization_and_default() {
        let config = CacheConfig {
            http_cache_path: Some("./http-cache".into()),
        };
        assert_toml_snapshot!(config, @"http_cache_path = './http-cache'");
    }

    #[test]
    fn deserialization() {
        let config: CacheConfig = toml::from_str(
            r#"
         http_cache_path = './http-cache'
    "#,
        )
        .unwrap();
        assert_eq!(
            config,
            CacheConfig {
                http_cache_path: Some("./http-cache".into()),
            }
        );
    }
}
