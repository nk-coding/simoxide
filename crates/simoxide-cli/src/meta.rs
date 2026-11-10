//! `--model-metadata` and `--runtime-metadata` (docs/guide/runtime-metadata.md): the model file
//! of the run's model, and per invocation the deployment at its start, its changes, the policy
//! state and the measurement series' metadata (`simoxide_sim::inspect`).

use super::Args;
use serde_json::Value;
use simoxide_sim::inspect::{Deployment, Recorder};
use simoxide_sim::{ChangeHost, CompiledModel};
use std::path::Path;

/// The SPD model of a run, for the target groups of its deployment.
#[cfg(feature = "spd")]
pub type Groups<'a> = Option<&'a simoxide_spd::SpdModel>;
#[cfg(not(feature = "spd"))]
pub type Groups<'a> = Option<&'a std::convert::Infallible>;

/// The deployment of `h`'s current model, with the target groups of `spd`.
pub fn deployment<H: ChangeHost + ?Sized>(h: &H, spd: Groups) -> Deployment {
    #[cfg(feature = "spd")]
    {
        simoxide_spd::inspect::deployment(h, spd)
    }
    #[cfg(not(feature = "spd"))]
    {
        let _ = spd;
        simoxide_sim::inspect::deployment(h.compiled_now(), &h.base_compiled().model)
    }
}

/// The model file of `cm` (the model a run starts from), with the scaling definitions of `spd`.
#[cfg(feature = "spd")]
pub fn model_file(
    cm: &CompiledModel,
    spd: Option<&simoxide_spd::Spd>,
    hpa: Option<&simoxide_spd::hpa::Hpa>,
) -> Value {
    simoxide_spd::inspect::model_file(cm, spd, hpa)
}

#[cfg(not(feature = "spd"))]
pub fn model_file(cm: &CompiledModel) -> Value {
    let d = simoxide_sim::inspect::deployment(cm, &cm.model);
    simoxide_sim::inspect::model_file(cm, &d, Value::Null)
}

/// Writes `--model-metadata` (if given) and returns the model file's hash for the runtime file
/// (if `--runtime-metadata` is given).
pub fn write_model(a: &Args, file: impl FnOnce() -> Value) -> Result<Option<String>, String> {
    if a.model_metadata.is_none() && a.runtime_metadata.is_none() {
        return Ok(None);
    }
    let v = file();
    if let Some(p) = &a.model_metadata {
        write_json(p, &v)?;
    }
    Ok(a.runtime_metadata
        .as_ref()
        .and(v["hash"].as_str().map(String::from)))
}

/// Starts the record of the deployment (`--runtime-metadata`), at the restore or the start.
pub fn begin<H: ChangeHost + ?Sized>(
    a: &Args,
    h: &H,
    spd: Groups,
    policies: impl FnOnce() -> Value,
) -> Option<Recorder> {
    a.runtime_metadata.as_ref()?;
    Some(Recorder::start(
        deployment(h, spd),
        h.now_ns(),
        h.model_generation(),
        policies(),
    ))
}

/// Records a change step of the resume (`--change`, `--transform`) applied to `h`.
pub fn record_step<H: ChangeHost + ?Sized>(
    rec: Option<&mut Recorder>,
    h: &H,
    spd: Groups,
    cause: Value,
) {
    if let Some(r) = rec {
        r.record(
            deployment(h, spd),
            h.now_ns(),
            h.model_generation(),
            cause,
            Value::Null,
        );
    }
}

/// What the runtime file needs from the end of the invocation.
pub struct End<'a> {
    /// The run's model now (with the elements that left it).
    pub cm: &'a CompiledModel,
    /// The model the run started from.
    pub base: &'a CompiledModel,
    pub series: &'a [simoxide_sim::ir::SeriesDef],
    pub time_ns: i64,
    pub generation: u32,
    pub policies: Value,
    /// The state id of the snapshot written, if any.
    pub state: Option<&'a str>,
    pub paused: bool,
}

/// Writes `--runtime-metadata` from `rec` and the end `e`.
pub fn finish(a: &Args, rec: Option<Recorder>, e: End) -> Result<(), String> {
    let (Some(rec), Some(p)) = (rec, &a.runtime_metadata) else {
        return Ok(());
    };
    let run = serde_json::json!({
        "parent": a.parent.as_ref().map(|(id, _)| id.as_str()),
        "state": e.state,
        "status": if e.paused { "paused" } else { "finished" },
    });
    let v = rec.into_json(
        a.model_hash.as_deref().unwrap_or_default(),
        run,
        e.cm,
        e.base,
        e.series,
        e.time_ns,
        e.generation,
        e.policies,
    );
    write_json(p, &v)
}

/// Writes `v` to `p` (compact JSON, a final newline).
fn write_json(p: &Path, v: &Value) -> Result<(), String> {
    let mut text = serde_json::to_string(v).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
}
