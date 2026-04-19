#![deny(warnings)]

// Minimal library target exposing just enough of Retrack's internals for the
// in-workspace perf harness at `benches/js-runtime-perf`. The main server
// binary at `src/main.rs` still owns the full module tree.
pub mod config {
    mod js_runtime_config;
    pub use js_runtime_config::JsRuntimeConfig;
}

pub mod js_runtime;
