use serde_derive::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's target for JSON API.
#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JsonApiTarget;
