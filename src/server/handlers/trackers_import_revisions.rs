use crate::{config::TrackersConfig, error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, web};
use retrack_types::trackers::{TrackerDataRevisionImportParams, TrackerDataRevisionImportResult};
use tracing::error;
use uuid::Uuid;

pub fn service(trackers_config: &TrackersConfig) -> actix_web::Resource {
    web::resource("/api/trackers/{tracker_id}/revisions/_import")
        .app_data(
            web::JsonConfig::default()
                .limit(trackers_config.max_import_body_size.as_u64() as usize),
        )
        .route(web::post().to(trackers_import_revisions))
}

/// Imports multiple data revisions for a tracker in bulk.
#[utoipa::path(
    method(post),
    path = "/api/trackers/{tracker_id}/revisions/_import",
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID.")
    ),
    request_body = Vec<TrackerDataRevisionImportParams>,
    responses(
        (status = OK, description = "Import result with counts.", body = TrackerDataRevisionImportResult),
    )
)]
pub async fn trackers_import_revisions(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
    params: web::Json<Vec<TrackerDataRevisionImportParams>>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .import_tracker_data_revisions(*tracker_id, params.into_inner())
        .await
    {
        Ok(result) => Ok(HttpResponse::Ok().json(result)),
        Err(err) => {
            error!(tracker.id = %tracker_id, "Failed to import tracker data revisions: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_import_revisions::service, server_state::tests::mock_server_state,
        },
        tests::{TrackerCreateParamsBuilder, mock_config},
    };
    use actix_web::{
        App,
        body::MessageBody,
        http::Method,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::{
        TrackerDataRevisionImportParams, TrackerDataRevisionImportResult, TrackerDataValue,
    };
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use time::OffsetDateTime;

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_import_revisions(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(service(&mock_config()?.trackers)),
        )
        .await;

        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("test-tracker").build())
            .await?;

        let revisions = vec![
            TrackerDataRevisionImportParams {
                data: TrackerDataValue::new(json!("data-1")),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
            TrackerDataRevisionImportParams {
                data: TrackerDataValue::new(json!("data-2")),
                created_at: OffsetDateTime::from_unix_timestamp(946720900)?,
            },
        ];

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/_import",
                tracker.id
            ))
            .method(Method::POST)
            .set_json(&revisions)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let bytes = response.into_body().try_into_bytes().unwrap();
        let body = from_utf8(&bytes)?;
        let result: TrackerDataRevisionImportResult = serde_json::from_str(body)?;
        assert_eq!(result.imported, 2);
        assert_eq!(result.skipped, 0);

        // Verify revisions were stored.
        let stored = server_state
            .api
            .trackers()
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(stored.len(), 2);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn skips_duplicate_timestamps(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(service(&mock_config()?.trackers)),
        )
        .await;

        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("test-tracker").build())
            .await?;

        let revisions = vec![
            TrackerDataRevisionImportParams {
                data: TrackerDataValue::new(json!("data-1")),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
            TrackerDataRevisionImportParams {
                data: TrackerDataValue::new(json!("data-duplicate")),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/_import",
                tracker.id
            ))
            .method(Method::POST)
            .set_json(&revisions)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let bytes = response.into_body().try_into_bytes().unwrap();
        let body = from_utf8(&bytes)?;
        let result: TrackerDataRevisionImportResult = serde_json::from_str(body)?;
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 1);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn returns_error_for_nonexistent_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(service(&mock_config()?.trackers)),
        )
        .await;

        let fake_id = uuid::Uuid::now_v7();
        let revisions = vec![TrackerDataRevisionImportParams {
            data: TrackerDataValue::new(json!("data-1")),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        }];

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{fake_id}/revisions/_import"
            ))
            .method(Method::POST)
            .set_json(&revisions)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn handles_empty_import(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(service(&mock_config()?.trackers)),
        )
        .await;

        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("test-tracker").build())
            .await?;

        let revisions: Vec<TrackerDataRevisionImportParams> = vec![];

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/_import",
                tracker.id
            ))
            .method(Method::POST)
            .set_json(&revisions)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let bytes = response.into_body().try_into_bytes().unwrap();
        let body = from_utf8(&bytes)?;
        let result: TrackerDataRevisionImportResult = serde_json::from_str(body)?;
        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 0);

        Ok(())
    }
}
