mod cache_config;
mod components_config;
mod database_config;
mod js_runtime_config;
mod raw_config;
mod scheduler_jobs_config;
mod smtp_config;
mod trackers_config;

use url::Url;

pub use self::{
    cache_config::CacheConfig, components_config::ComponentsConfig,
    database_config::DatabaseConfig, js_runtime_config::JsRuntimeConfig, raw_config::RawConfig,
    scheduler_jobs_config::SchedulerJobsConfig, smtp_config::SmtpConfig,
    trackers_config::TrackersConfig,
};

/// Main server config.
#[derive(Clone, Debug)]
pub struct Config {
    /// External/public URL through which service is being accessed.
    pub public_url: Url,
    /// Database configuration.
    pub db: DatabaseConfig,
    /// Configuration for the various caches Retrack relies on.
    pub cache: CacheConfig,
    /// Configuration for the SMTP functionality.
    pub smtp: Option<SmtpConfig>,
    /// Configuration for the components that are deployed separately.
    pub components: ComponentsConfig,
    /// Configuration for the scheduler jobs.
    pub scheduler: SchedulerJobsConfig,
    /// Configuration for the trackers.
    pub trackers: TrackersConfig,
    /// Configuration for the embedded JS Runtime.
    pub js_runtime: JsRuntimeConfig,
}

impl AsRef<Config> for Config {
    fn as_ref(&self) -> &Config {
        self
    }
}

impl From<RawConfig> for Config {
    fn from(raw_config: RawConfig) -> Self {
        Self {
            public_url: raw_config.public_url,
            db: raw_config.db,
            cache: raw_config.cache,
            smtp: raw_config.smtp,
            components: raw_config.components,
            scheduler: raw_config.scheduler,
            trackers: raw_config.trackers,
            js_runtime: raw_config.js_runtime,
        }
    }
}

#[cfg(test)]
pub mod tests {
    pub use crate::config::smtp_config::SmtpCatchAllConfig;
    use crate::config::{Config, RawConfig, SmtpConfig};
    use insta::assert_debug_snapshot;
    use regex::Regex;

    #[test]
    fn conversion_from_raw_config() {
        let raw_config = RawConfig {
            smtp: Some(SmtpConfig {
                username: "test@retrack.dev".to_string(),
                password: "password".to_string(),
                address: "smtp.retrack.dev".to_string(),
                catch_all: Some(SmtpCatchAllConfig {
                    recipient: "test@retrack.dev".to_string(),
                    text_matcher: Regex::new(r"test").unwrap(),
                }),
            }),
            ..Default::default()
        };

        assert_debug_snapshot!(Config::from(raw_config), @r###"
        Config {
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
                    7676,
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
                password: None,
                max_connections: 100,
            },
            cache: CacheConfig {
                http_cache_path: None,
            },
            smtp: Some(
                SmtpConfig {
                    username: "test@retrack.dev",
                    password: "password",
                    address: "smtp.retrack.dev",
                    catch_all: Some(
                        SmtpCatchAllConfig {
                            recipient: "test@retrack.dev",
                            text_matcher: Regex(
                                "test",
                            ),
                        },
                    ),
                },
            ),
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
                trackers_schedule: "0/10 * * * * *",
                trackers_run: "0/10 * * * * *",
                tasks_run: "0/30 * * * * *",
            },
            trackers: TrackersConfig {
                max_revisions: 30,
                schedules: None,
                min_schedule_interval: 10s,
                restrict_to_public_urls: true,
                max_script_size: Byte(
                    4096,
                ),
            },
            js_runtime: JsRuntimeConfig {
                max_heap_size: 10485760,
                max_script_execution_time: 10s,
                channel_buffer_size: 10,
            },
        }
        "###);
    }
}
