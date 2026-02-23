use handlebars::JsonRender;
use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
use serde_json::{Value as JSONValue, json};
use similar::TextDiff;

/// Pretty prints the web page content revision data.
fn tracker_data_revision_pretty_print(data: &str) -> anyhow::Result<String> {
    let json_data = serde_json::from_str::<JSONValue>(data)?;
    Ok(
        if json_data.is_object() || json_data.is_array() || json_data.is_null() {
            serde_json::to_string_pretty(&json_data)?
        } else {
            json_data.render()
        },
    )
}

/// Takes multiple web page content revisions and calculates the diff. The `context_radius`
/// controls how many unchanged lines are included around each changed hunk.
pub fn tracker_data_revisions_diff(
    revisions: Vec<TrackerDataRevision>,
    context_radius: usize,
) -> anyhow::Result<Vec<TrackerDataRevision>> {
    if revisions.len() < 2 {
        return Ok(revisions);
    }

    let mut revisions_diff = Vec::with_capacity(revisions.len());
    let mut peekable_revisions = revisions.into_iter().peekable();
    while let Some(current_revision) = peekable_revisions.next() {
        if let Some(previous_revision) = peekable_revisions.peek() {
            let current_value =
                tracker_data_revision_pretty_print(&current_revision.data.value().to_string())?;
            let previous_value =
                tracker_data_revision_pretty_print(&previous_revision.data.value().to_string())?;

            revisions_diff.push(TrackerDataRevision {
                data: TrackerDataValue::new(json!(
                    TextDiff::from_lines(&previous_value, &current_value)
                        .unified_diff()
                        .context_radius(context_radius)
                        .missing_newline_hint(false)
                        .to_string()
                )),
                ..current_revision
            });
        } else {
            revisions_diff.push(current_revision);
        }
    }

    Ok(revisions_diff.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use crate::trackers::tracker_data_revisions_diff::tracker_data_revisions_diff;
    use insta::assert_debug_snapshot;
    use retrack_types::trackers::{
        DEFAULT_DIFF_CONTEXT_RADIUS, TrackerDataRevision, TrackerDataValue,
    };
    use serde_json::json;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn correctly_calculates_data_revision_diff() -> anyhow::Result<()> {
        let revisions = vec![
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: TrackerDataValue::new(json!("\"Hello New World\"")),
                created_at: OffsetDateTime::from_unix_timestamp(946720801)?,
            },
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: TrackerDataValue::new(json!("\"Hello World\"")),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        let diff = tracker_data_revisions_diff(revisions, DEFAULT_DIFF_CONTEXT_RADIUS)?;
        assert_debug_snapshot!(diff, @r###"
        [
            TrackerDataRevision {
                id: 00000000-0000-0000-0000-000000000002,
                tracker_id: 00000000-0000-0000-0000-000000000002,
                data: TrackerDataValue {
                    original: String("@@ -1 +1 @@\n-\"Hello World\"\n+\"Hello New World\"\n"),
                    mods: None,
                },
                created_at: 2000-01-01 10:00:01.0 +00:00:00,
            },
            TrackerDataRevision {
                id: 00000000-0000-0000-0000-000000000001,
                tracker_id: 00000000-0000-0000-0000-000000000002,
                data: TrackerDataValue {
                    original: String("\"Hello World\""),
                    mods: None,
                },
                created_at: 2000-01-01 10:00:00.0 +00:00:00,
            },
        ]
        "###);

        Ok(())
    }

    #[test]
    fn correctly_calculates_data_revision_diff_for_json() -> anyhow::Result<()> {
        let revisions = vec![TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            data: TrackerDataValue::new(json!({ "property": "one", "secondProperty": "two" })),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        }];

        let diff = tracker_data_revisions_diff(revisions, DEFAULT_DIFF_CONTEXT_RADIUS)?;
        assert_debug_snapshot!(diff, @r###"
        [
            TrackerDataRevision {
                id: 00000000-0000-0000-0000-000000000001,
                tracker_id: 00000000-0000-0000-0000-000000000002,
                data: TrackerDataValue {
                    original: Object {
                        "property": String("one"),
                        "secondProperty": String("two"),
                    },
                    mods: None,
                },
                created_at: 2000-01-01 10:00:00.0 +00:00:00,
            },
        ]
        "###);

        let revisions = vec![
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000003"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: TrackerDataValue::new(
                    json!({ "property": "one", "secondProperty": "two", "thirdProperty": "three" }),
                ),
                created_at: OffsetDateTime::from_unix_timestamp(946720802)?,
            },
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: TrackerDataValue::new(json!({ "property": "one" })),
                created_at: OffsetDateTime::from_unix_timestamp(946720801)?,
            },
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: TrackerDataValue::new(json!({ "property": "one", "secondProperty": "two" })),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        let diff = tracker_data_revisions_diff(revisions, DEFAULT_DIFF_CONTEXT_RADIUS)?;
        assert_debug_snapshot!(diff, @r###"
        [
            TrackerDataRevision {
                id: 00000000-0000-0000-0000-000000000003,
                tracker_id: 00000000-0000-0000-0000-000000000002,
                data: TrackerDataValue {
                    original: String("@@ -1,3 +1,5 @@\n {\n-  \"property\": \"one\"\n+  \"property\": \"one\",\n+  \"secondProperty\": \"two\",\n+  \"thirdProperty\": \"three\"\n }\n"),
                    mods: None,
                },
                created_at: 2000-01-01 10:00:02.0 +00:00:00,
            },
            TrackerDataRevision {
                id: 00000000-0000-0000-0000-000000000002,
                tracker_id: 00000000-0000-0000-0000-000000000002,
                data: TrackerDataValue {
                    original: String("@@ -1,4 +1,3 @@\n {\n-  \"property\": \"one\",\n-  \"secondProperty\": \"two\"\n+  \"property\": \"one\"\n }\n"),
                    mods: None,
                },
                created_at: 2000-01-01 10:00:01.0 +00:00:00,
            },
            TrackerDataRevision {
                id: 00000000-0000-0000-0000-000000000001,
                tracker_id: 00000000-0000-0000-0000-000000000002,
                data: TrackerDataValue {
                    original: Object {
                        "property": String("one"),
                        "secondProperty": String("two"),
                    },
                    mods: None,
                },
                created_at: 2000-01-01 10:00:00.0 +00:00:00,
            },
        ]
        "###);

        Ok(())
    }

    #[test]
    fn respects_custom_context_radius() -> anyhow::Result<()> {
        let revisions = vec![
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
                data: TrackerDataValue::new(json!({
                    "a": 1, "b": 2, "c": 3, "d": 4, "e": 5,
                    "f": 6, "g": 7, "h": "changed", "i": 9, "j": 10
                })),
                created_at: OffsetDateTime::from_unix_timestamp(946720801)?,
            },
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
                data: TrackerDataValue::new(json!({
                    "a": 1, "b": 2, "c": 3, "d": 4, "e": 5,
                    "f": 6, "g": 7, "h": 8, "i": 9, "j": 10
                })),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        // With context_radius=0, only changed lines + hunk headers are shown.
        let diff_zero = tracker_data_revisions_diff(revisions.clone(), 0)?;
        let diff_zero_str = diff_zero[0].data.value().as_str().unwrap();
        assert!(
            !diff_zero_str.contains("\"a\": 1"),
            "context_radius=0 should not include distant unchanged lines"
        );
        assert!(
            diff_zero_str.contains("-  \"h\": 8"),
            "context_radius=0 should include the removed line"
        );
        assert!(
            diff_zero_str.contains("+  \"h\": \"changed\""),
            "context_radius=0 should include the added line"
        );

        // With a very large context_radius, all lines are included.
        let diff_large = tracker_data_revisions_diff(revisions, 10000)?;
        let diff_large_str = diff_large[0].data.value().as_str().unwrap();
        assert!(
            diff_large_str.contains("\"a\": 1"),
            "large context_radius should include all lines"
        );

        Ok(())
    }
}
