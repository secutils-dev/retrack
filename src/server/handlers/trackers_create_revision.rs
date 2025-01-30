use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{post, web, HttpResponse};
use retrack_types::trackers::TrackerDataRevision;
use tracing::error;
use uuid::Uuid;

/// Tries to create a new revision for a tracker with the specified ID. If revision is equal to the
/// previous one, it will not be created, and the previous revision will be returned.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID.")
    ),
    responses(
        (status = OK, description = "Newly created data revision.", body = TrackerDataRevision)
    )
)]
#[post("/api/trackers/{tracker_id}/revisions")]
pub async fn trackers_create_revision(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.create_tracker_data_revision(*tracker_id).await {
        Ok(revision) => Ok(HttpResponse::Ok().json(revision)),
        Err(err) => {
            error!("Failed to create tracker data revision: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_create_revision::trackers_create_revision,
            server_state::tests::{mock_server_state, mock_server_state_with_config},
        },
        tests::{mock_config, TrackerCreateParamsBuilder, WebScraperContentRequest},
    };
    use actix_web::{
        body::MessageBody,
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use url::Url;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_create_tracker_data(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;
        let server_state = web::Data::new(mock_server_state_with_config(pool, config).await?);

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create_revision),
        )
        .await;

        let content_one = TrackerDataValue::new(json!("\"rev_1\""));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
                        .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_one.value());
        });

        // Add tracker data revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .method(Method::POST)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let revision = serde_json::from_slice::<TrackerDataRevision>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        let saved_revision = trackers_api
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(saved_revision[0].id, revision.id);
        assert_eq!(saved_revision[0].data, revision.data);

        content_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_unknown_trackers(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create_revision),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .method(Method::POST)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""Tracker ('00000000-0000-0000-0000-000000000001') is not found.""###);

        Ok(())
    }
}
