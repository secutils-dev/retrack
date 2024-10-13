use crate::{
    config::Config,
    database::Database,
    js_runtime::JsRuntime,
    network::{DnsResolver, EmailTransport, Network},
};
use handlebars::Handlebars;

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
}

impl<DR: DnsResolver, ET: EmailTransport> AsRef<Api<DR, ET>> for Api<DR, ET> {
    fn as_ref(&self) -> &Self {
        self
    }
}
