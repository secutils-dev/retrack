use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Default page size used when the client does not provide one.
pub const DEFAULT_TRACKERS_PAGE_SIZE: u32 = 15;
/// Maximum page size a client can request. Larger values are clamped.
pub const MAX_TRACKERS_PAGE_SIZE: u32 = 100;

/// Sort direction for tracker lists.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// Sortable tracker fields.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrackersListSort {
    Name,
    CreatedAt,
    #[default]
    UpdatedAt,
    Enabled,
    ScheduledAt,
    LastRanAt,
}

/// Resolved pagination parameters ready for use by the API/database layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTrackersListParams {
    pub offset: i64,
    pub limit: i64,
    pub sort: TrackersListSort,
    pub order: SortOrder,
    pub query: Option<String>,
}

/// Parameters for getting a paginated list of trackers.
#[derive(Deserialize, Default, Debug, Clone, PartialEq, Eq, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct TrackersListParams {
    /// List of tags to filter trackers by.
    #[param(max_items = 10, min_length = 1, max_length = 50)]
    #[serde(default, rename = "tag")]
    pub tags: Vec<String>,
    /// Zero-based page index. Defaults to `0`.
    #[serde(default)]
    #[param(minimum = 0)]
    pub page: Option<u32>,
    /// Number of items per page. Defaults to 15, clamped to a maximum of 100.
    #[serde(default)]
    #[param(minimum = 1, maximum = 100)]
    pub page_size: Option<u32>,
    /// Field to sort by. Defaults to `updatedAt`.
    #[serde(default)]
    pub sort: Option<TrackersListSort>,
    /// Sort direction. Defaults to `asc`.
    #[serde(default)]
    pub order: Option<SortOrder>,
    /// Free-text query matched case-insensitively against the tracker name.
    #[serde(default)]
    pub q: Option<String>,
}

impl TrackersListParams {
    pub fn resolve(&self) -> ResolvedTrackersListParams {
        let page = self.page.unwrap_or_default();
        let page_size = self
            .page_size
            .unwrap_or(DEFAULT_TRACKERS_PAGE_SIZE)
            .clamp(1, MAX_TRACKERS_PAGE_SIZE);
        ResolvedTrackersListParams {
            offset: i64::from(page) * i64::from(page_size),
            limit: i64::from(page_size),
            sort: self.sort.unwrap_or_default(),
            order: self.order.unwrap_or_default(),
            query: self.q.as_deref().and_then(|q| {
                let trimmed = q.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(escape_like(trimmed))
                }
            }),
        }
    }
}

/// Escapes `ILIKE` wildcard metacharacters so user search input is matched literally.
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use crate::trackers::{
        DEFAULT_TRACKERS_PAGE_SIZE, MAX_TRACKERS_PAGE_SIZE, SortOrder, TrackersListParams,
        TrackersListSort,
    };

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackersListParams>(r#"{}"#)?,
            TrackersListParams::default()
        );

        assert_eq!(
            serde_json::from_str::<TrackersListParams>(
                r#"
{
    "tag": ["tag_one", "tag_two"],
    "page": 2,
    "pageSize": 50,
    "sort": "scheduledAt",
    "order": "desc",
    "q": "tracker"
}
          "#
            )?,
            TrackersListParams {
                tags: vec!["tag_one".to_string(), "tag_two".to_string()],
                page: Some(2),
                page_size: Some(50),
                sort: Some(TrackersListSort::ScheduledAt),
                order: Some(SortOrder::Desc),
                q: Some("tracker".to_string()),
            }
        );

        Ok(())
    }

    #[test]
    fn resolves_defaults() {
        let resolved = TrackersListParams::default().resolve();
        assert_eq!(resolved.offset, 0);
        assert_eq!(resolved.limit, i64::from(DEFAULT_TRACKERS_PAGE_SIZE));
        assert_eq!(resolved.sort, TrackersListSort::UpdatedAt);
        assert_eq!(resolved.order, SortOrder::Asc);
        assert!(resolved.query.is_none());
    }

    #[test]
    fn clamps_page_size_and_escapes_query() {
        let resolved = TrackersListParams {
            page: Some(3),
            page_size: Some(MAX_TRACKERS_PAGE_SIZE + 1),
            q: Some("  50%_off\\now  ".to_string()),
            ..Default::default()
        }
        .resolve();
        assert_eq!(resolved.offset, 3 * i64::from(MAX_TRACKERS_PAGE_SIZE));
        assert_eq!(resolved.limit, i64::from(MAX_TRACKERS_PAGE_SIZE));
        assert_eq!(resolved.query.as_deref(), Some("50\\%\\_off\\\\now"));

        let resolved = TrackersListParams {
            page_size: Some(0),
            q: Some("   ".to_string()),
            ..Default::default()
        }
        .resolve();
        assert_eq!(resolved.limit, 1);
        assert!(resolved.query.is_none());
    }
}
