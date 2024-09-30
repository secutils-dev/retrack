use crate::config::DatabaseConfig;
use anyhow::Context;
use sqlx::{PgPool, Pool, Postgres};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: Pool<Postgres>,
}

/// Common methods for the primary database, extensions are implemented separately in every module.
impl Database {
    /// Opens database "connection".
    pub async fn create(pool: PgPool) -> anyhow::Result<Self> {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .with_context(|| "Failed to migrate database")?;

        Ok(Database { pool })
    }

    /// Returns current UTC time, truncated to microseconds to match the database precision.
    pub fn utc_now() -> anyhow::Result<OffsetDateTime> {
        let now = OffsetDateTime::now_utc();
        Ok(now.replace_nanosecond(now.microsecond() * 1000)?)
    }

    /// Constructs full database connection URL based on the provided database config.
    pub fn connection_url(config: &DatabaseConfig) -> String {
        format!(
            "postgres://{}@{}:{}/{}",
            if let Some(ref password) = config.password {
                format!(
                    "{}:{}",
                    urlencoding::encode(&config.username),
                    urlencoding::encode(password)
                )
            } else {
                config.username.clone()
            },
            config.host,
            config.port,
            urlencoding::encode(&config.name)
        )
    }
}

impl AsRef<Database> for Database {
    fn as_ref(&self) -> &Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::{config::DatabaseConfig, database::Database};

    #[test]
    fn correctly_constructs_connection_url() {
        assert_eq!(
            Database::connection_url(&DatabaseConfig {
                host: "retrack.db.local".to_string(),
                username: "retrack_db_user".to_string(),
                ..Default::default()
            }),
            "postgres://retrack_db_user@retrack.db.local:5432/retrack"
        );

        assert_eq!(
            Database::connection_url(&DatabaseConfig {
                host: "retrack.db.local".to_string(),
                username: "retrack_db_user".to_string(),
                password: Some("db_password".to_string()),
                ..Default::default()
            }),
            "postgres://retrack_db_user:db_password@retrack.db.local:5432/retrack"
        );
    }
}
