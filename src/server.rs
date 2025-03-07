mod handlers;
mod server_state;

use crate::{
    api::Api, database::Database, network::Network, scheduler::Scheduler,
    templates::create_templates,
};
use actix_cors::Cors;
use actix_web::{App, HttpServer, Result, middleware, web};
use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tracing::info;
use tracing_actix_web::TracingLogger;
use utoipa::OpenApi;
use utoipa_rapidoc::RapiDoc;

use crate::{
    config::{Config, RawConfig},
    js_runtime::JsRuntime,
    server::handlers::RetrackOpenApi,
};
pub use server_state::{GetStatusParams, SchedulerStatus, ServerState, Status};

pub async fn run(raw_config: RawConfig) -> Result<(), anyhow::Error> {
    let database = Database::create(
        PgPoolOptions::new()
            .max_connections(raw_config.db.max_connections)
            .connect(&Database::connection_url(&raw_config.db))
            .await?,
    )
    .await?;

    let http_port = raw_config.port;
    let js_runtime = JsRuntime::init_platform(&raw_config.js_runtime)?;
    let config = Config::from(raw_config);
    let network = Network::create(&config)?;

    let api = Arc::new(Api::new(
        config,
        database,
        network,
        create_templates()?,
        js_runtime,
    ));

    api.migrate().await?;

    let scheduler = Scheduler::start(api.clone()).await?;
    let state = web::Data::new(ServerState::new(api, scheduler));
    let http_server = HttpServer::new(move || {
        App::new()
            .wrap(middleware::Compat::new(TracingLogger::default()))
            .wrap(middleware::Compat::new(middleware::Compress::default()))
            .wrap(middleware::NormalizePath::trim())
            .app_data(state.clone())
            .service(RapiDoc::with_openapi(
                "/api-docs/openapi.json",
                RetrackOpenApi::openapi(),
            ))
            .service(handlers::status_get::status_get)
            .service(handlers::trackers_list::trackers_list)
            .service(handlers::trackers_get::trackers_get)
            .service(handlers::trackers_create::trackers_create)
            .service(handlers::trackers_update::trackers_update)
            .service(handlers::trackers_remove::trackers_remove)
            .service(handlers::trackers_bulk_remove::trackers_bulk_remove)
            .service(handlers::trackers_list_revisions::trackers_list_revisions)
            .service(handlers::trackers_create_revision::trackers_create_revision)
            .service(handlers::trackers_clear_revisions::trackers_clear_revisions)
            .service(handlers::trackers_get_revision::trackers_get_revision)
            .service(handlers::trackers_remove_revision::trackers_remove_revision)
            .wrap(Cors::permissive())
    });

    let http_server_url = format!("0.0.0.0:{}", http_port);
    let http_server = http_server
        .bind(&http_server_url)
        .with_context(|| format!("Failed to bind to {http_server_url}."))?;

    info!("Retrack API server is available at http://{http_server_url}");

    http_server
        .run()
        .await
        .context("Failed to run Retrack API server.")
}
