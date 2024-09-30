use serde_derive::{Deserialize, Serialize};

/// Configuration for the database connection.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DatabaseConfig {
    /// Name of the database to connect to.
    pub name: String,
    /// Hostname to use to connect to the database.
    pub host: String,
    /// Port to use to connect to the database.
    pub port: u16,
    /// Username to use to connect to the database.
    pub username: String,
    /// Optional password to use to connect to the database.
    pub password: Option<String>,
    /// Defines a maximum number of connections allowed.
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            name: "retrack".to_string(),
            host: "localhost".to_string(),
            port: 5432,
            username: "postgres".to_string(),
            password: None,
            max_connections: 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::DatabaseConfig;
    use insta::{assert_debug_snapshot, assert_toml_snapshot};

    #[test]
    fn serialization() {
        let config = DatabaseConfig::default();
        assert_toml_snapshot!(config, @r###"
        name = 'retrack'
        host = 'localhost'
        port = 5432
        username = 'postgres'
        max_connections = 100
        "###);

        let config = DatabaseConfig {
            password: Some("password".to_string()),
            ..Default::default()
        };
        assert_toml_snapshot!(config, @r###"
        name = 'retrack'
        host = 'localhost'
        port = 5432
        username = 'postgres'
        password = 'password'
        max_connections = 100
        "###);
    }

    #[test]
    fn deserialization() {
        let config: DatabaseConfig = toml::from_str(
            r#"
        name = 'retrack'
        username = 'postgres'
        password = 'password'
        host = 'localhost'
        port = 5432
        max_connections = 1000
    "#,
        )
        .unwrap();
        assert_debug_snapshot!(config, @r###"
        DatabaseConfig {
            name: "retrack",
            host: "localhost",
            port: 5432,
            username: "postgres",
            password: Some(
                "password",
            ),
            max_connections: 1000,
        }
        "###);
    }
}
