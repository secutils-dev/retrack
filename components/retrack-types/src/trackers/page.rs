use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A page of results returned by a paginated list endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Page<T: ToSchema> {
    /// Items on the current page.
    pub items: Vec<T>,
    /// Total number of items matching the filter across all pages.
    pub total: i64,
}

impl<T: ToSchema> Page<T> {
    pub fn new(items: Vec<T>, total: i64) -> Self {
        Self { items, total }
    }
}
