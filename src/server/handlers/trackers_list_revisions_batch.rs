use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, post, web};
use retrack_types::trackers::{TrackerDataRevision, TrackerListRevisionsBatchParams};
use tracing::error;

/// Gets data revisions for multiple trackers in a single request.
#[utoipa::path(
    tags = ["trackers"],
    request_body = TrackerListRevisionsBatchParams,
    responses(
        (status = OK, description = "A map of tracker ID to data revisions.", body = std::collections::HashMap<String, Vec<TrackerDataRevision>>),
        (status = BAD_REQUEST, description = "Cannot list revisions with the specified parameters.")
    )
)]
#[post("/api/trackers/revisions")]
pub async fn trackers_list_revisions_batch(
    state: web::Data<ServerState>,
    params: web::Json<TrackerListRevisionsBatchParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .get_tracker_data_revisions_batch(params.into_inner())
        .await
    {
        Ok(revisions) => Ok(HttpResponse::Ok().json(revisions)),
        Err(err) => {
            error!("Failed to retrieve batch tracker data revisions: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_list_revisions_batch::trackers_list_revisions_batch,
            server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        App,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
    use sqlx::PgPool;
    use std::collections::HashMap;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[sqlx::test]
    async fn returns_empty_map_for_empty_ids(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state)
                .service(trackers_list_revisions_batch),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::post()
                .uri("/api/trackers/revisions")
                .set_json(serde_json::json!({"trackerIds": []}))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), 200);
        let body: HashMap<Uuid, Vec<TrackerDataRevision>> =
            actix_web::test::read_body_json(response).await;
        assert!(body.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn returns_revisions_grouped_by_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let trackers_api = server_state.api.trackers();

        let tracker_a = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-a").build())
            .await?;
        let tracker_b = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-b").build())
            .await?;

        let db = server_state.api.db.trackers();
        db.insert_tracker_data_revision(&TrackerDataRevision {
            id: uuid::uuid!("00000000-0000-0000-0000-000000000010"),
            tracker_id: tracker_a.id,
            data: TrackerDataValue::new(serde_json::json!("rev-a-1")),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        })
        .await?;
        db.insert_tracker_data_revision(&TrackerDataRevision {
            id: uuid::uuid!("00000000-0000-0000-0000-000000000020"),
            tracker_id: tracker_b.id,
            data: TrackerDataValue::new(serde_json::json!("rev-b-1")),
            created_at: OffsetDateTime::from_unix_timestamp(946720810)?,
        })
        .await?;

        let app = init_service(
            App::new()
                .app_data(server_state)
                .service(trackers_list_revisions_batch),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::post()
                .uri("/api/trackers/revisions")
                .set_json(serde_json::json!({
                    "trackerIds": [tracker_a.id, tracker_b.id]
                }))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), 200);
        let body: HashMap<Uuid, Vec<TrackerDataRevision>> =
            actix_web::test::read_body_json(response).await;
        assert_eq!(body.len(), 2);
        assert_eq!(body[&tracker_a.id].len(), 1);
        assert_eq!(
            body[&tracker_a.id][0].data.original(),
            &serde_json::json!("rev-a-1")
        );
        assert_eq!(body[&tracker_b.id].len(), 1);
        assert_eq!(
            body[&tracker_b.id][0].data.original(),
            &serde_json::json!("rev-b-1")
        );

        Ok(())
    }
}
