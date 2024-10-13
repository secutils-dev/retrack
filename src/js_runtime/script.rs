use retrack_types::trackers::{
    ConfiguratorScriptArgs, ConfiguratorScriptResult, ExtractorScriptArgs, ExtractorScriptResult,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_bytes::ByteBuf;
use std::marker::PhantomData;
use tokio::sync::oneshot;

/// Represents all supported types of JS scripts that can be executed.
pub enum Script {
    /// A script to configure request for API tracker target.
    ApiTargetConfigurator(ScriptDefinition<ConfiguratorScriptArgs, ConfiguratorScriptResult>),
    /// A script to preprocess response for API tracker target.
    ApiTargetExtractor(ScriptDefinition<ExtractorScriptArgs, ExtractorScriptResult>),
    /// A custom script for ad-hoc purposes.
    Custom(ScriptDefinition<ByteBuf, ByteBuf>),
}

type ScriptResultSender<Result> = oneshot::Sender<anyhow::Result<Option<Result>>>;
pub struct ScriptDefinition<ScriptArgs: Serialize, ScriptResult: DeserializeOwned> {
    /// Source code of the scripts to execute.
    pub src: String,
    /// Optional list of the script arguments.
    pub args: Option<ScriptArgs>,
    /// A channel to send result of the script execution from JS runtime thread to the main thread.
    pub result: ScriptResultSender<ScriptResult>,
}

pub trait ScriptBuilder<ScriptArgs, ScriptResult> {
    fn build(
        self,
        src: impl Into<String>,
        result_tx: ScriptResultSender<ScriptResult>,
    ) -> (Script, PhantomData<ScriptArgs>);
}

/// Implementation for API target "configurator" script.
impl ScriptBuilder<ConfiguratorScriptArgs, ConfiguratorScriptResult> for ConfiguratorScriptArgs {
    fn build(
        self,
        src: impl Into<String>,
        result: ScriptResultSender<ConfiguratorScriptResult>,
    ) -> (Script, PhantomData<ConfiguratorScriptArgs>) {
        (
            Script::ApiTargetConfigurator(ScriptDefinition {
                src: src.into(),
                args: Some(self),
                result,
            }),
            PhantomData,
        )
    }
}

/// Implementation for API target "extractor" script.
impl ScriptBuilder<ExtractorScriptArgs, ExtractorScriptResult> for ExtractorScriptArgs {
    fn build(
        self,
        src: impl Into<String>,
        result: ScriptResultSender<ExtractorScriptResult>,
    ) -> (Script, PhantomData<ExtractorScriptArgs>) {
        (
            Script::ApiTargetExtractor(ScriptDefinition {
                src: src.into(),
                args: Some(self),
                result,
            }),
            PhantomData,
        )
    }
}

/// Implementation for API target "extractor" script. Args are represented as `Uint8Array` in JS
/// and should be decoded with `Deno.core.decode`. Result is converted to `Uint8Array` as well with
/// `Deno.core.encode`.
impl ScriptBuilder<ByteBuf, ByteBuf> for Option<ByteBuf> {
    fn build(
        self,
        src: impl Into<String>,
        result: ScriptResultSender<ByteBuf>,
    ) -> (Script, PhantomData<ByteBuf>) {
        (
            Script::Custom(ScriptDefinition {
                src: src.into(),
                args: self,
                result,
            }),
            PhantomData,
        )
    }
}
