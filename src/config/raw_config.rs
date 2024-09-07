use crate::config::{
    database_config::DatabaseConfig, ComponentsConfig, SchedulerJobsConfig, SmtpConfig,
    TrackersConfig,
};
use figment::{providers, providers::Format, value, Figment, Metadata, Profile, Provider};
use serde_derive::{Deserialize, Serialize};
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
    /// Configuration for the components that are deployed separately.
    pub components: ComponentsConfig,
    /// Configuration for the scheduler jobs.
    pub scheduler: SchedulerJobsConfig,
    /// Configuration for the trackers.
    pub trackers: TrackersConfig,
    /// Configuration for the SMTP functionality.
    pub smtp: Option<SmtpConfig>,
}

impl RawConfig {
    /// Reads the configuration from the file (TOML) and merges it with the default values.
    pub fn read_from_file(path: &str) -> anyhow::Result<Self> {
        Ok(Figment::from(RawConfig::default())
            .merge(providers::Toml::file(path))
            .merge(providers::Env::prefixed("RETRACK_").split("__"))
            .extract()?)
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
            smtp: None,
        }
    }
}

impl Provider for RawConfig {
    fn metadata(&self) -> Metadata {
        Metadata::named("Retrack API server main configuration")
    }

    fn data(&self) -> Result<value::Map<Profile, value::Dict>, figment::Error> {
        providers::Serialized::defaults(Self::default()).data()
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

        [components]
        web_scraper_url = 'http://localhost:7272/'

        [scheduler]
        trackers_schedule = '0/10 * * * * * *'
        trackers_run = '0/10 * * * * * *'
        tasks_run = '0/30 * * * * * *'

        [trackers]
        max_revisions = 30
        min_schedule_interval = 10000
        restrict_to_public_urls = true
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

        [components]
        web_scraper_url = 'http://localhost:7272/'

        [scheduler]
        trackers_schedule = '0 * * * * * *'
        trackers_run = '0 * * * * * *'
        tasks_run = '0/30 * * * * * *'

        [trackers]
        schedules = ['@hourly']
        max_revisions = 11
        min_schedule_interval = 10_000
        restrict_to_public_urls = true
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
                trackers_schedule: Schedule {
                    source: "0 * * * * * *",
                    fields: ScheduleFields {
                        years: Years {
                            ordinals: None,
                        },
                        days_of_week: DaysOfWeek {
                            ordinals: None,
                        },
                        months: Months {
                            ordinals: None,
                        },
                        days_of_month: DaysOfMonth {
                            ordinals: None,
                        },
                        hours: Hours {
                            ordinals: None,
                        },
                        minutes: Minutes {
                            ordinals: None,
                        },
                        seconds: Seconds {
                            ordinals: Some(
                                {
                                    0,
                                },
                            ),
                        },
                    },
                },
                trackers_run: Schedule {
                    source: "0 * * * * * *",
                    fields: ScheduleFields {
                        years: Years {
                            ordinals: None,
                        },
                        days_of_week: DaysOfWeek {
                            ordinals: None,
                        },
                        months: Months {
                            ordinals: None,
                        },
                        days_of_month: DaysOfMonth {
                            ordinals: None,
                        },
                        hours: Hours {
                            ordinals: None,
                        },
                        minutes: Minutes {
                            ordinals: None,
                        },
                        seconds: Seconds {
                            ordinals: Some(
                                {
                                    0,
                                },
                            ),
                        },
                    },
                },
                tasks_run: Schedule {
                    source: "0/30 * * * * * *",
                    fields: ScheduleFields {
                        years: Years {
                            ordinals: None,
                        },
                        days_of_week: DaysOfWeek {
                            ordinals: None,
                        },
                        months: Months {
                            ordinals: None,
                        },
                        days_of_month: DaysOfMonth {
                            ordinals: None,
                        },
                        hours: Hours {
                            ordinals: None,
                        },
                        minutes: Minutes {
                            ordinals: None,
                        },
                        seconds: Seconds {
                            ordinals: Some(
                                {
                                    0,
                                    30,
                                },
                            ),
                        },
                    },
                },
            },
            trackers: TrackersConfig {
                max_revisions: 11,
                schedules: Some(
                    {
                        "@hourly",
                    },
                ),
                min_schedule_interval: 10s,
                restrict_to_public_urls: true,
            },
            smtp: None,
        }
        "###);
    }
}
