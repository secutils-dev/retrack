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
}

impl AsRef<Database> for Database {
    fn as_ref(&self) -> &Self {
        self
    }
}
