use crate::config::{
    database_config::DatabaseConfig, CacheConfig, ComponentsConfig, JsRuntimeConfig,
    SchedulerJobsConfig, SmtpConfig, TrackersConfig,
};
use figment::{providers, providers::Format, Figment};
use serde::{Deserialize, Serialize};
use url::Url;

/// Raw configuration structure that is used to read the configuration from the file.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RawConfig {
    /// Defines a TCP port to listen on.
    pub port: u16,
    /// External/public URL through which service is being accessed.
    pub public_url: Url,
    /// Database configuration.
    pub db: DatabaseConfig,
    /// Defines various caches related settings.
    pub cache: CacheConfig,
    /// Configuration for the components that are deployed separately.
    pub components: ComponentsConfig,
    /// Configuration for the scheduler jobs.
    pub scheduler: SchedulerJobsConfig,
    /// Configuration for the trackers.
    pub trackers: TrackersConfig,
    /// Configuration for the SMTP functionality.
    pub smtp: Option<SmtpConfig>,
    /// Configuration for the embedded JS Runtime.
    pub js_runtime: JsRuntimeConfig,
}

impl RawConfig {
    /// Reads the configuration from the file (TOML) and merges it with the default values.
    pub fn read_from_file(path: &str) -> anyhow::Result<Self> {
        Ok(
            Figment::from(providers::Serialized::defaults(Self::default()))
                .merge(providers::Toml::file(path))
                .merge(providers::Env::prefixed("RETRACK_").split("__"))
                .extract()?,
        )
    }
}

impl Default for RawConfig {
    fn default() -> Self {
        let port = 7676;
        Self {
            port,
            db: DatabaseConfig::default(),
            public_url: Url::parse(&format!("http://localhost:{port}"))
                .expect("Cannot parse public URL parameter."),
            components: ComponentsConfig::default(),
            scheduler: SchedulerJobsConfig::default(),
            trackers: TrackersConfig::default(),
            js_runtime: JsRuntimeConfig::default(),
            smtp: None,
            cache: CacheConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::RawConfig;
    use insta::{assert_debug_snapshot, assert_toml_snapshot};

    #[test]
    fn serialization_and_default() {
        let default_config = RawConfig::default();

        assert_toml_snapshot!(default_config, @r###"
        port = 7676
        public_url = 'http://localhost:7676/'

        [db]
        name = 'retrack'
        host = 'localhost'
        port = 5432
        username = 'postgres'
        max_connections = 100

        [cache]

        [components]
        web_scraper_url = 'http://localhost:7272/'

        [scheduler]
        tasks_run = '0/30 * * * * *'
        trackers_schedule = '0/10 * * * * *'

        [trackers]
        max_revisions = 30
        max_timeout = 300000
        min_schedule_interval = 10000
        min_retry_interval = 60000
        restrict_to_public_urls = true
        max_script_size = '4 KiB'

        [js_runtime]
        max_heap_size = 10485760
        max_script_execution_time = 10000
        channel_buffer_size = 10
        "###);
    }

    #[test]
    fn deserialization() {
        let config: RawConfig = toml::from_str(
            r#"
        port = 7070
        public_url = 'http://localhost:7070/'

        [db]
        name = 'retrack'
        schema = 'retrack'
        username = 'postgres'
        password = 'password'
        host = 'localhost'
        port = 5432
        max_connections = 1000

        [cache]
        http_cache_path = './http-cache'

        [components]
        web_scraper_url = 'http://localhost:7272/'

        [js_runtime]
        max_heap_size = 20485760
        max_script_execution_time = 20000
        channel_buffer_size = 200

        [scheduler]
        trackers_schedule = '0 * * * * * *'
        trackers_run = '0 * * * * * *'
        tasks_run = '0/30 * * * * * *'

        [trackers]
        schedules = ['@hourly']
        max_revisions = 11
        max_timeout = 300_000
        min_schedule_interval = 10_000
        min_retry_interval = 60_000
        restrict_to_public_urls = true
        max_script_size = '4 KiB'
    "#,
        )
        .unwrap();

        assert_debug_snapshot!(config, @r###"
        RawConfig {
            port: 7070,
            public_url: Url {
                scheme: "http",
                cannot_be_a_base: false,
                username: "",
                password: None,
                host: Some(
                    Domain(
                        "localhost",
                    ),
                ),
                port: Some(
                    7070,
                ),
                path: "/",
                query: None,
                fragment: None,
            },
            db: DatabaseConfig {
                name: "retrack",
                host: "localhost",
                port: 5432,
                username: "postgres",
                password: Some(
                    "password",
                ),
                max_connections: 1000,
            },
            cache: CacheConfig {
                http_cache_path: Some(
                    "./http-cache",
                ),
            },
            components: ComponentsConfig {
                web_scraper_url: Url {
                    scheme: "http",
                    cannot_be_a_base: false,
                    username: "",
                    password: None,
                    host: Some(
                        Domain(
                            "localhost",
                        ),
                    ),
                    port: Some(
                        7272,
                    ),
                    path: "/",
                    query: None,
                    fragment: None,
                },
            },
            scheduler: SchedulerJobsConfig {
                tasks_run: "0/30 * * * * * *",
                trackers_schedule: "0 * * * * * *",
            },
            trackers: TrackersConfig {
                max_revisions: 11,
                max_timeout: 300s,
                schedules: Some(
                    {
                        "@hourly",
                    },
                ),
                min_schedule_interval: 10s,
                min_retry_interval: 60s,
                restrict_to_public_urls: true,
                max_script_size: Byte(
                    4096,
                ),
            },
            smtp: None,
            js_runtime: JsRuntimeConfig {
                max_heap_size: 20485760,
                max_script_execution_time: 20s,
                channel_buffer_size: 200,
            },
        }
        "###);
    }
}
