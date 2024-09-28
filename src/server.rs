mod handlers;
mod server_state;

use crate::{
    api::Api,
    database::Database,
    network::{Network, TokioDnsResolver},
    scheduler::Scheduler,
    templates::create_templates,
};
use actix_cors::Cors;
use actix_web::{middleware, web, App, HttpServer, Result};
use anyhow::Context;
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    Tokio1Executor,
};
use sqlx::postgres::PgPoolOptions;
use std::{str::FromStr, sync::Arc};
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

#[tokio::main]
pub async fn run(raw_config: RawConfig) -> Result<(), anyhow::Error> {
    let db_url = format!(
        "postgres://{}@{}:{}/{}",
        if let Some(ref password) = raw_config.db.password {
            format!(
                "{}:{}",
                urlencoding::encode(&raw_config.db.username),
                urlencoding::encode(password)
            )
        } else {
            raw_config.db.username.clone()
        },
        raw_config.db.host,
        raw_config.db.port,
        urlencoding::encode(&raw_config.db.name)
    );
    let database = Database::create(
        PgPoolOptions::new()
            .max_connections(100)
            .connect(&db_url)
            .await?,
    )
    .await?;

    let email_transport = if let Some(ref smtp_config) = raw_config.smtp {
        if let Some(ref catch_all_config) = smtp_config.catch_all {
            Mailbox::from_str(catch_all_config.recipient.as_str())
                .with_context(|| "Cannot parse SMTP catch-all recipient.")?;
        }

        AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp_config.address)?
            .credentials(Credentials::new(
                smtp_config.username.clone(),
                smtp_config.password.clone(),
            ))
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::unencrypted_localhost()
    };

    let http_port = raw_config.port;
    let api = Arc::new(Api::new(
        Config::from(raw_config),
        database,
        Network::new(TokioDnsResolver::create(), email_transport),
        create_templates()?,
    ));

    JsRuntime::init_platform();

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
            .service(handlers::trackers_list_revisions::trackers_list_revisions)
            .service(handlers::trackers_clear_revisions::trackers_clear_revisions)
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
        .with_context(|| "Failed to run Retrack API server.")
}
