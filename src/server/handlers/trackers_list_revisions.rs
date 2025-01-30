use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{get, web, HttpResponse};
use retrack_types::trackers::{TrackerDataRevision, TrackerListRevisionsParams};
use tracing::error;
use uuid::Uuid;

/// Gets a list of revisions for a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
        TrackerListRevisionsParams
    ),
    responses(
        (status = OK, description = "A list of currently active trackers.", body = [TrackerDataRevision]),
        (status = BAD_REQUEST, description = "Cannot list revisions for a tracker with the specified parameters.")
    )
)]
#[get("/api/trackers/{tracker_id}/revisions")]
pub async fn trackers_list_revisions(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
    params: web::Query<TrackerListRevisionsParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .get_tracker_data_revisions(*tracker_id, params.into_inner())
        .await
    {
        Ok(revisions) => Ok(HttpResponse::Ok().json(revisions)),
        Err(err) => {
            error!("Failed to retrieve tracker data revisions: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_list_revisions::trackers_list_revisions,
            server_state::tests::mock_server_state,
        },
        tests::{tracker_data_revisions_diff, TrackerCreateParamsBuilder},
    };
    use actix_web::{
        body::MessageBody,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use insta::assert_debug_snapshot;
    use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_list_tracker_data(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_revisions),
        )
        .await;

        // No data yet.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "[]"
        );

        // Add tracker data revision.
        let trackers_db = server_state.api.db.trackers();
        let data_revision_one = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: tracker.id,
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            data: TrackerDataValue::new(json!("\"some-data\"")),
        };
        trackers_db
            .insert_tracker_data_revision(&data_revision_one)
            .await?;
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let revisions = serde_json::from_slice::<Vec<TrackerDataRevision>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert_eq!(revisions, vec![data_revision_one.clone()]);

        // Add another revision
        let mut data = TrackerDataValue::new(json!("\"some-new-data\""));
        data.add_mod(json!("\"some-other-data\""));
        let data_revision_two = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000002"),
            tracker_id: tracker.id,
            created_at: OffsetDateTime::from_unix_timestamp(946720900)?,
            data,
        };
        trackers_db
            .insert_tracker_data_revision(&data_revision_two)
            .await?;
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let revisions = serde_json::from_slice::<Vec<TrackerDataRevision>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert_eq!(
            revisions,
            vec![data_revision_two.clone(), data_revision_one.clone()]
        );

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions?size=10",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let revisions = serde_json::from_slice::<Vec<TrackerDataRevision>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert_eq!(
            revisions,
            vec![data_revision_two.clone(), data_revision_one.clone()]
        );

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions?size=1",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let revisions = serde_json::from_slice::<Vec<TrackerDataRevision>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert_eq!(revisions, vec![data_revision_two.clone()]);

        // Calculate the difference between the two revisions
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions?calculateDiff=true",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let revisions = serde_json::from_slice::<Vec<TrackerDataRevision>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert_eq!(
            revisions,
            tracker_data_revisions_diff(vec![data_revision_two.clone(), data_revision_one])?
        );

        // Does not calculate the difference, if there is only one revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions?calculateDiff=true&size=1",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let revisions = serde_json::from_slice::<Vec<TrackerDataRevision>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert_eq!(revisions, vec![data_revision_two]);

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_unknown_trackers(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_revisions),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""Tracker ('00000000-0000-0000-0000-000000000001') is not found.""###);

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_zero_size(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_revisions),
        )
        .await;

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions?size=0",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""Query deserialize error: invalid value: integer `0`, expected a nonzero usize""###);

        Ok(())
    }
}
