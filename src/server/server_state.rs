mod scheduler_status;
mod status;
mod status_get_params;

pub use self::{
    scheduler_status::SchedulerStatus, status::Status, status_get_params::GetStatusParams,
};
use crate::{
    api::Api,
    network::{DnsResolver, TokioDnsResolver},
    scheduler::Scheduler,
};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ServerState<DR: DnsResolver = TokioDnsResolver> {
    pub api: Arc<Api<DR>>,
    pub scheduler: RwLock<Scheduler<DR>>,
    /// Version of the API server.
    version: String,
}

impl<DR: DnsResolver> ServerState<DR> {
    pub fn new(api: Arc<Api<DR>>, scheduler: Scheduler<DR>) -> Self {
        Self {
            api,
            scheduler: RwLock::new(scheduler),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Gets the status of the server.
    pub async fn status(&self) -> anyhow::Result<Status<'_>> {
        Ok(Status {
            version: self.version.as_str(),
            scheduler: self.scheduler.write().await.status().await?,
        })
    }
}

#[cfg(test)]
pub mod tests {
    use crate::{
        api::Api,
        config::Config,
        database::Database,
        js_runtime::JsRuntime,
        network::Network,
        scheduler::Scheduler,
        server::ServerState,
        templates::create_templates,
        tests::{mock_config, mock_scheduler},
    };
    use sqlx::PgPool;
    use std::sync::Arc;

    pub async fn mock_server_state(pool: PgPool) -> anyhow::Result<ServerState> {
        mock_server_state_with_config(pool, mock_config()?).await
    }

    pub async fn mock_server_state_with_config(
        pool: PgPool,
        config: Config,
    ) -> anyhow::Result<ServerState> {
        // We should use a real network implementation in tests that rely on `AppState` being
        // extracted from `HttpRequest`, as types should match for the extraction to work.
        let network = Network::create(&config)?;
        let js_runtime = JsRuntime::init_platform(&config.js_runtime)?;
        let api = Arc::new(Api::new(
            config,
            Database::create(pool.clone()).await?,
            network,
            create_templates()?,
            js_runtime,
        ));

        let scheduler = Scheduler {
            inner_scheduler: mock_scheduler(&pool).await?,
            api: api.clone(),
        };

        Ok(ServerState::new(api, scheduler))
    }
}
