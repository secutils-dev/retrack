#![deny(warnings)]

mod api;
mod config;
mod database;
mod error;
mod js_runtime;
mod network;
mod scheduler;
mod server;
mod tasks;
mod templates;
mod trackers;

use crate::config::{Config, RawConfig};
use anyhow::anyhow;
use clap::{crate_authors, crate_description, crate_version, value_parser, Arg, Command};
use std::env;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    dotenvy::dotenv().ok();

    if env::var("RUST_LOG_FORMAT").is_ok_and(|format| format == "json") {
        tracing_subscriber::fmt().json().flatten_event(true).init();
    } else {
        tracing_subscriber::fmt::init();
    }

    let matches = Command::new("Retrack API server.")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(
            Arg::new("CONFIG")
                .env("RETRACK_CONFIG")
                .short('c')
                .long("config")
                .default_value("retrack.toml")
                .help("Path to the Retrack configuration file."),
        )
        .arg(
            Arg::new("PORT")
                .env("RETRACK_PORT")
                .short('p')
                .long("port")
                .value_parser(value_parser!(u16))
                .help("Defines a TCP port to listen on."),
        )
        .get_matches();

    let mut raw_config = RawConfig::read_from_file(
        matches
            .get_one::<String>("CONFIG")
            .ok_or_else(|| anyhow!("<CONFIG> argument is not provided."))?,
    )?;

    // CLI argument takes precedence.
    if let Some(port) = matches.get_one::<u16>("PORT") {
        raw_config.port = *port;
    }

    info!(config = ?raw_config, "Retrack raw configuration.");

    server::run(raw_config).await
}

#[cfg(test)]
mod tests {
    use crate::{
        api::Api,
        config::{ComponentsConfig, Config, SchedulerJobsConfig, SmtpConfig},
        database::Database,
        network::{DnsResolver, Network},
    };
    use lettre::transport::stub::AsyncStubTransport;
    use std::{ops::Add, time::Duration};
    use time::OffsetDateTime;
    use trust_dns_resolver::proto::rr::Record;
    use url::Url;

    pub use crate::{config::tests::*, network::tests::*, scheduler::tests::*, trackers::tests::*};
    use crate::{
        config::{CacheConfig, DatabaseConfig, JsRuntimeConfig, TrackersConfig},
        js_runtime::JsRuntime,
        templates::create_templates,
    };
    use sqlx::{postgres::PgDatabaseError, PgPool};

    pub fn to_database_error(err: anyhow::Error) -> anyhow::Result<Box<PgDatabaseError>> {
        Ok(err
            .downcast::<sqlx::Error>()?
            .into_database_error()
            .unwrap()
            .downcast::<PgDatabaseError>())
    }

    pub fn mock_config() -> anyhow::Result<Config> {
        Ok(Config {
            public_url: Url::parse("http://localhost:1234")?,
            db: DatabaseConfig::default(),
            cache: CacheConfig {
                http_cache_path: Some("./target/http-cache".into()),
            },
            smtp: Some(SmtpConfig {
                username: "dev@retrack.dev".to_string(),
                password: "password".to_string(),
                address: "localhost".to_string(),
                catch_all: None,
            }),
            components: ComponentsConfig::default(),
            scheduler: SchedulerJobsConfig {
                trackers_schedule: "0 * 0 * * *".to_string(),
                trackers_run: "0 * 1 * * *".to_string(),
                tasks_run: "0 * 2 * * *".to_string(),
            },
            trackers: TrackersConfig {
                restrict_to_public_urls: false,
                ..Default::default()
            },
            js_runtime: JsRuntimeConfig::default(),
        })
    }

    pub fn mock_network() -> Network<MockResolver, AsyncStubTransport> {
        Network::new(MockResolver::new(), AsyncStubTransport::new_ok())
    }

    pub fn mock_network_with_records<const N: usize>(
        records: Vec<Record>,
    ) -> Network<MockResolver<N>, AsyncStubTransport> {
        Network::new(
            MockResolver::new_with_records::<N>(records),
            AsyncStubTransport::new_ok(),
        )
    }

    pub async fn mock_api(pool: PgPool) -> anyhow::Result<Api<MockResolver, AsyncStubTransport>> {
        mock_api_with_config(pool, mock_config()?).await
    }

    pub async fn mock_api_with_config(
        pool: PgPool,
        config: Config,
    ) -> anyhow::Result<Api<MockResolver, AsyncStubTransport>> {
        let js_runtime = JsRuntime::init_platform(&config.js_runtime)?;
        Ok(Api::new(
            config,
            Database::create(pool).await?,
            mock_network(),
            create_templates()?,
            js_runtime,
        ))
    }

    pub async fn mock_api_with_network<DR: DnsResolver>(
        pool: PgPool,
        network: Network<DR, AsyncStubTransport>,
    ) -> anyhow::Result<Api<DR, AsyncStubTransport>> {
        let config = mock_config()?;
        let js_runtime = JsRuntime::init_platform(&config.js_runtime)?;
        Ok(Api::new(
            config,
            Database::create(pool).await?,
            network,
            create_templates()?,
            js_runtime,
        ))
    }

    pub fn mock_schedule_in_sec(secs: u64) -> String {
        format!(
            "{} * * * * *",
            OffsetDateTime::now_utc()
                .add(Duration::from_secs(secs))
                .second()
        )
    }

    pub fn mock_schedule_in_secs(secs: &[u64]) -> String {
        format!(
            "{} * * * * *",
            secs.iter()
                .map(|secs| {
                    OffsetDateTime::now_utc()
                        .add(Duration::from_secs(*secs))
                        .second()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}
