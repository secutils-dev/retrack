use crate::{
    config::Config,
    database::Database,
    js_runtime::JsRuntime,
    network::{DnsResolver, EmailTransport, Network},
};
use handlebars::Handlebars;
use tracing::info;

pub struct Api<DR: DnsResolver, ET: EmailTransport> {
    pub db: Database,
    pub config: Config,
    pub network: Network<DR, ET>,
    pub templates: Handlebars<'static>,
    pub js_runtime: JsRuntime,
}

impl<DR: DnsResolver, ET: EmailTransport> Api<DR, ET> {
    /// Instantiates APIs collection with the specified config and datastore.
    pub fn new(
        config: Config,
        database: Database,
        network: Network<DR, ET>,
        templates: Handlebars<'static>,
        js_runtime: JsRuntime,
    ) -> Self {
        Self {
            config,
            db: database,
            network,
            templates,
            js_runtime,
        }
    }

    /// Migrates trackers to the latest API interface version, if needed. The migration is as simple
    /// as loading all trackers from the database using the latest or previous API interface
    /// versions and saving the tracker to the database using the latest API interface version.
    pub async fn migrate(&self) -> anyhow::Result<()> {
        info!("Migration is not needed.");
        Ok(())
    }
}

impl<DR: DnsResolver, ET: EmailTransport> AsRef<Api<DR, ET>> for Api<DR, ET> {
    fn as_ref(&self) -> &Self {
        self
    }
}
