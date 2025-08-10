use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, post, web};
use retrack_types::trackers::{Tracker, TrackerCreateParams};
use tracing::error;

/// Creates a new tracker with the specified parameters.
#[utoipa::path(
    tags = ["trackers"],
    request_body = TrackerCreateParams,
    responses(
        (status = 200, description = "Tracker was successfully created.", body = Tracker),
        (status = BAD_REQUEST, description = "Cannot create a tracker with the specified properties.")
    )
)]
#[post("/api/trackers")]
pub async fn trackers_create(
    state: web::Data<ServerState>,
    params: web::Json<TrackerCreateParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.create_tracker(params.into_inner()).await {
        Ok(tracker) => Ok(HttpResponse::Created().json(tracker)),
        Err(err) => {
            error!("Failed to create tracker: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::{Config, TrackersConfig},
        server::{
            handlers::trackers_create::trackers_create,
            server_state::tests::{mock_server_state, mock_server_state_with_config},
        },
        tests::mock_config,
    };
    use actix_web::{
        App,
        body::MessageBody,
        http::Method,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{ExtractorEngine, PageTarget, TrackerTarget},
    };
    use serde_json::json;
    use sqlx::PgPool;
    use std::{str::from_utf8, time::Duration};

    #[sqlx::test]
    async fn can_create_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::POST)
                .set_json(json!({
                    "name": "my-minimal-tracker".to_string(),
                    "target": {
                        "type": "page",
                        "extractor": "export async function execute(p) { await p.goto('https://retrack.dev'); return await p.content(); }"
                    },
                }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 201);

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 1);
        assert_eq!(trackers[0].name, "my-minimal-tracker");
        assert!(trackers[0].enabled);
        assert_eq!(trackers[0].config.revisions, 3);
        assert!(trackers[0].config.timeout.is_none());
        assert_eq!(
            trackers[0].target,
            TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev'); return await p.content(); }".to_string(),
                params: None,
                engine: None,
                user_agent: None,
                accept_invalid_certificates: false,
            })
        );

        assert_eq!(
            serde_json::to_string(&trackers[0])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_create_tracker_with_optional_params(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::POST)
                .set_json(json!({
                    "name": "my-minimal-tracker".to_string(),
                    "enabled": false,
                    "target": {
                        "type": "page",
                        "extractor": "export async function execute(p) { await p.goto('https://retrack.dev'); return await p.content(); }",
                        "params": { "param": "value" },
                        "engine": { "type": "chromium" },
                        "userAgent": "Retrack/1.0.0",
                        "acceptInvalidCertificates": true
                    },
                    "config": {
                        "revisions": 5,
                        "timeout": 5000,
                        "headers": {
                            "cookie": "my-cookie"
                        },
                        "job": {
                            "schedule": "@daily",
                            "retryStrategy": {
                                "type": "constant",
                                "interval": 500000,
                                "maxAttempts": 5
                            }
                        }
                    }
                }))
                .to_request(),
        )
        .await;

        let status = response.status();
        let body = response.into_body().try_into_bytes().unwrap();

        assert_eq!(status, 201);

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 1);
        assert_eq!(trackers[0].name, "my-minimal-tracker");
        assert!(!trackers[0].enabled);
        assert_eq!(trackers[0].config.revisions, 5);
        assert_eq!(
            trackers[0].config.timeout,
            Some(Duration::from_millis(5000))
        );
        assert_eq!(
            trackers[0].target,
            TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev'); return await p.content(); }".to_string(),
                params: Some(json!({ "param": "value" })),
                engine: Some(ExtractorEngine::Chromium),
                user_agent: Some("Retrack/1.0.0".to_string()),
                accept_invalid_certificates: true,
            })
        );
        assert_eq!(
            trackers[0].config.job,
            Some(SchedulerJobConfig {
                schedule: "@daily".to_string(),
                retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                    interval: Duration::from_secs(500),
                    max_attempts: 5
                })
            })
        );
        assert_eq!(serde_json::to_string(&trackers[0])?, from_utf8(&body)?);

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_invalid_params(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(
            mock_server_state_with_config(
                pool,
                Config {
                    trackers: TrackersConfig {
                        restrict_to_public_urls: true,
                        ..Default::default()
                    },
                    ..mock_config()?
                },
            )
            .await?,
        );
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::POST)
                .set_json(json!({
                    "name": "my-minimal-tracker".to_string(),
                    "target": {
                        "type": "api",
                        "requests": [{ "url": "https://127.0.0.1/app" }]
                    },
                    "config": {
                        "revisions": 5,
                        "timeout": 5000
                    }
                }))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), 400);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            r###"Tracker target URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/app."###
        );
        assert!(
            server_state
                .api
                .trackers()
                .get_trackers(Default::default())
                .await?
                .is_empty()
        );

        Ok(())
    }
}
