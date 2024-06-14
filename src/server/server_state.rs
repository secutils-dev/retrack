mod status;

use crate::{
    api::Api,
    network::{DnsResolver, EmailTransport, TokioDnsResolver},
};
use lettre::{AsyncSmtpTransport, Tokio1Executor};
use std::sync::{Arc, RwLock};

pub use self::status::Status;

pub struct ServerState<
    DR: DnsResolver = TokioDnsResolver,
    ET: EmailTransport = AsyncSmtpTransport<Tokio1Executor>,
> {
    pub status: RwLock<Status>,
    pub api: Arc<Api<DR, ET>>,
}

impl<DR: DnsResolver, ET: EmailTransport> ServerState<DR, ET> {
    pub fn new(api: Arc<Api<DR, ET>>) -> Self {
        Self {
            status: RwLock::new(Status {
                version: env!("CARGO_PKG_VERSION").to_string(),
            }),
            api,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use crate::{
        api::Api,
        config::Config,
        database::Database,
        network::{Network, TokioDnsResolver},
        server::ServerState,
        templates::create_templates,
        tests::mock_config,
    };
    use lettre::{AsyncSmtpTransport, Tokio1Executor};
    use sqlx::PgPool;
    use std::sync::Arc;

    pub async fn mock_server_state(pool: PgPool) -> anyhow::Result<ServerState> {
        mock_server_state_with_config(pool, mock_config()?).await
    }

    pub async fn mock_server_state_with_config(
        pool: PgPool,
        config: Config,
    ) -> anyhow::Result<ServerState> {
        let api = Arc::new(Api::new(
            config,
            Database::create(pool).await?,
            // We should use a real network implementation in tests that rely on `AppState` being
            // extracted from `HttpRequest`, as types should match for the extraction to work.
            Network::new(
                TokioDnsResolver::create(),
                AsyncSmtpTransport::<Tokio1Executor>::unencrypted_localhost(),
            ),
            create_templates()?,
        ));

        Ok(ServerState::new(api))
    }
}
