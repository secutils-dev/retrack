use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{put, web, HttpResponse};
use retrack_types::trackers::{Tracker, TrackerUpdateParams};
use tracing::error;
use uuid::Uuid;

/// Updates an existing tracker with the specified parameters.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
    ),
    request_body = TrackerUpdateParams,
    responses(
        (status = 200, description = "Tracker was successfully updated.", body = Tracker),
        (status = BAD_REQUEST, description = "Cannot update a tracker with the specified properties.")
    )
)]
#[put("/api/trackers/{tracker_id}")]
pub async fn trackers_update(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
    params: web::Json<TrackerUpdateParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .update_tracker(*tracker_id, params.into_inner())
        .await
    {
        Ok(tracker) => Ok(HttpResponse::Ok().json(tracker)),
        Err(err) => {
            error!("Failed to update tracker: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::{Config, TrackersConfig},
        server::{
            handlers::trackers_update::trackers_update,
            server_state::tests::{mock_server_state, mock_server_state_with_config},
        },
        tests::{mock_config, TrackerCreateParamsBuilder},
    };
    use actix_web::{
        body::MessageBody,
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use insta::assert_debug_snapshot;
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;

    #[sqlx::test]
    async fn can_update_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_update),
        )
        .await;

        // Create tracker.
        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;
        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 1);

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!("https://retrack.dev/api/trackers/{}", tracker.id))
                .method(Method::PUT)
                .set_json(json!({
                    "name": "new_name_one".to_string(),
                    "enabled": false,
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
                    },
                    "tags": ["tag_two"],
                    "actions": [{ "type": "log" }, { "type": "webhook", "url": "https://retrack.dev" }],
                }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let tracker = server_state
            .api
            .trackers()
            .get_tracker(tracker.id)
            .await?
            .unwrap();
        assert_eq!(tracker.name, "new_name_one");
        assert!(!tracker.enabled);
        assert_eq!(tracker.tags, vec!["tag_two".to_string()]);
        assert_debug_snapshot!(tracker.config, @r###"
        TrackerConfig {
            revisions: 5,
            timeout: Some(
                5s,
            ),
            job: Some(
                SchedulerJobConfig {
                    schedule: "@daily",
                    retry_strategy: Some(
                        Constant {
                            interval: 500s,
                            max_attempts: 5,
                        },
                    ),
                },
            ),
        }
        "###);
        assert_debug_snapshot!(tracker.actions, @r###"
        [
            ServerLog(
                ServerLogAction {
                    formatter: None,
                },
            ),
            Webhook(
                WebhookAction {
                    url: Url {
                        scheme: "https",
                        cannot_be_a_base: false,
                        username: "",
                        password: None,
                        host: Some(
                            Domain(
                                "retrack.dev",
                            ),
                        ),
                        port: None,
                        path: "/",
                        query: None,
                        fragment: None,
                    },
                    method: None,
                    headers: None,
                    formatter: None,
                },
            ),
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_invalid_params(pool: PgPool) -> anyhow::Result<()> {
        // Create tracker.
        let server_state = web::Data::new(mock_server_state(pool.clone()).await?);
        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;
        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 1);

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
                .service(trackers_update),
        )
        .await;
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!("https://retrack.dev/api/trackers/{}", tracker.id))
                .method(Method::PUT)
                .set_json(json!({ "target": { "type": "api", "requests": [{ "url": "https://localhost/app" }] } }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            r###"{"message":"Tracker target URL must be either `http` or `https` and have a valid public reachable domain name, but received https://localhost/app."}"###
        );
        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers, vec![tracker]);

        Ok(())
    }
}
