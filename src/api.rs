use crate::{
    config::Config,
    database::Database,
    js_runtime::JsRuntime,
    network::{DnsResolver, Network},
};
use handlebars::Handlebars;
use tracing::info;

pub struct Api<DR: DnsResolver> {
    pub db: Database,
    pub config: Config,
    pub network: Network<DR>,
    pub templates: Handlebars<'static>,
    pub js_runtime: JsRuntime,
}

impl<DR: DnsResolver> Api<DR> {
    /// Instantiates APIs collection with the specified config and datastore.
    pub fn new(
        config: Config,
        database: Database,
        network: Network<DR>,
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
        let trackers = self.db.trackers().get_trackers(Default::default()).await?;
        info!(
            "Found {} trackers that will be attempted to migrate to new API interface version.",
            trackers.len()
        );

        for tracker in trackers.iter() {
            self.db.trackers().update_tracker(tracker).await?;
        }

        info!(
            "Migration completed successfully ({} trackers migrated).",
            trackers.len()
        );
        Ok(())
    }
}

impl<DR: DnsResolver> AsRef<Api<DR>> for Api<DR> {
    fn as_ref(&self) -> &Self {
        self
    }
}
