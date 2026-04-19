//! Helpers shared by every Retrack scenario: config defaults, script fixtures,
//! and a helper for building a realistic [`ExtractorScriptArgs`] context.

use http::{StatusCode, header::CONTENT_TYPE};
use retrack::{config::JsRuntimeConfig, js_runtime::ScriptConfig};
use retrack_types::trackers::{ExtractorScriptArgs, TargetResponse};
use serde_json::json;
use std::{collections::HashMap, time::Duration};

pub const TRIVIAL_JS: &str = include_str!("../../scripts/trivial.js");
pub const EXTRACTOR_JS: &str = include_str!("../../scripts/extractor.js");

/// Matches the production defaults in [`JsRuntimeConfig::default`]: 10 MiB
/// heap, 10 s execution limit, 10-slot mpsc channel. We intentionally reuse
/// the real defaults so recorded numbers approximate production behaviour.
pub fn runtime_config() -> JsRuntimeConfig {
    JsRuntimeConfig::default()
}

/// Per-script config applied to every iteration. Mirrors what
/// `ApiTargetExecutor` passes in a production extractor run.
pub fn script_config() -> ScriptConfig {
    ScriptConfig {
        max_heap_size: 10 * 1024 * 1024,
        max_execution_time: Duration::from_secs(10),
    }
}

/// Build an [`ExtractorScriptArgs`] whose first response carries a modest JSON
/// body - 16 items, just enough to exercise decode/parse/filter/encode without
/// letting serialisation dominate isolate startup.
pub fn extractor_args() -> ExtractorScriptArgs {
    let items: Vec<_> = (0..16)
        .map(|i| json!({ "id": i, "value": i * 3 + 7 }))
        .collect();
    let body = serde_json::to_vec(&json!({ "items": items })).expect("serialise items");

    let mut headers_map = HashMap::new();
    headers_map.insert(CONTENT_TYPE, "application/json".to_string());

    ExtractorScriptArgs {
        tags: vec!["perf".to_string()],
        previous_content: None,
        params: None,
        responses: Some(vec![TargetResponse {
            status: StatusCode::OK,
            headers: (&headers_map).try_into().expect("build headers"),
            body,
        }]),
    }
}
