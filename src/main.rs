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

use crate::config::RawConfig;
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

    // Install default crypto provider.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default RusTLS crypto provider.");

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
    use bytes::Bytes;
    use lettre::transport::smtp::authentication::Credentials;
    use std::{fs, ops::Add, path::PathBuf, time::Duration};
    use time::OffsetDateTime;
    use trust_dns_resolver::proto::rr::Record;
    use url::Url;

    pub use crate::{config::tests::*, network::tests::*, scheduler::tests::*, trackers::tests::*};
    use crate::{
        config::{CacheConfig, DatabaseConfig, JsRuntimeConfig, TrackersConfig},
        js_runtime::JsRuntime,
        network::{Smtp, SmtpTransport},
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

    pub fn mock_smtp_config(host: impl Into<String>, port: u16) -> SmtpConfig {
        SmtpConfig {
            username: "dev@retrack.dev".to_string(),
            password: "password".to_string(),
            host: host.into(),
            port: Some(port),
            no_tls: false,
            catch_all: None,
            throttle_delay: Duration::from_millis(1),
        }
    }

    pub fn mock_config() -> anyhow::Result<Config> {
        Ok(Config {
            public_url: Url::parse("http://localhost:1234")?,
            db: DatabaseConfig::default(),
            cache: CacheConfig {
                http_cache_path: Some("./target/http-cache".into()),
            },
            smtp: None,
            components: ComponentsConfig::default(),
            scheduler: SchedulerJobsConfig {
                trackers_schedule: "0 * 0 * * *".to_string(),
                tasks_run: "0 * 1 * * *".to_string(),
            },
            trackers: TrackersConfig {
                restrict_to_public_urls: false,
                ..Default::default()
            },
            js_runtime: JsRuntimeConfig::default(),
        })
    }

    pub fn mock_smtp(config: SmtpConfig) -> Smtp {
        Smtp::new(
            SmtpTransport::builder_dangerous(&config.host)
                .port(config.port.unwrap_or(25))
                .credentials(Credentials::new(
                    config.username.clone(),
                    config.password.clone(),
                ))
                .build(),
            config,
        )
    }

    pub fn mock_network() -> Network<MockResolver> {
        Network {
            resolver: MockResolver::new(),
            smtp: None,
        }
    }

    pub fn mock_network_with_records<const N: usize>(
        records: Vec<Record>,
    ) -> Network<MockResolver<N>> {
        Network {
            resolver: MockResolver::new_with_records::<N>(records),
            smtp: None,
        }
    }

    pub fn mock_network_with_smtp(smtp: Smtp) -> Network<MockResolver> {
        Network {
            resolver: MockResolver::new(),
            smtp: Some(smtp),
        }
    }

    pub async fn mock_api(pool: PgPool) -> anyhow::Result<Api<MockResolver>> {
        mock_api_with_config(pool, mock_config()?).await
    }

    pub async fn mock_api_with_config(
        pool: PgPool,
        config: Config,
    ) -> anyhow::Result<Api<MockResolver>> {
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
        network: Network<DR>,
    ) -> anyhow::Result<Api<DR>> {
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

    pub fn load_fixture(fixture_name: &str) -> anyhow::Result<Bytes> {
        let mut fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        fixture_path.push(format!("dev/fixtures/{fixture_name}"));
        Ok(fs::read(fixture_path).map(Bytes::from)?)
    }
}
