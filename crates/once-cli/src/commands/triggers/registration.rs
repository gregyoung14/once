use std::path::Path;

use anyhow::{Context, Result};
use once_cas::Digest;
use once_frontend::{Diagnostic, TriggerSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::storage;

const SCHEMA: &str = "once.trigger_activation.v1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Activation {
    schema: String,
    repository: String,
    pub generation: String,
    pub revision: String,
    manifest_digest: String,
    pub triggers: Vec<TriggerSpec>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Observation {
    pub event: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub revision: String,
}

#[derive(Debug, Serialize)]
pub(super) struct Selection {
    schema: &'static str,
    mode: &'static str,
    pub contract_generation: String,
    pub definition_revision: String,
    pub observation: Observation,
    pub trigger: TriggerSpec,
}

pub(crate) fn activate(workspace: &Path, revision: &str) -> Result<Value> {
    let mut lock = storage::lock(workspace)?;
    let _guard = lock
        .try_write()
        .context("another trigger operation owns this workspace")?;
    let repository = storage::repository(workspace)?;
    storage::revision(workspace, revision)?;
    let source = storage::git(workspace, &["show", &format!("{revision}:once.toml")])?;
    let source = storage::read_bounded(source.as_bytes())?;
    let triggers =
        once_frontend::load_triggers_toml_str("once.toml", std::str::from_utf8(&source)?)
            .map_err(declarations_error)?;
    let activation = Activation {
        schema: SCHEMA.to_string(),
        repository,
        generation: uuid::Uuid::now_v7().to_string(),
        revision: revision.to_string(),
        manifest_digest: Digest::of_bytes(&source).to_string(),
        triggers,
    };
    storage::atomic_write(
        &workspace.join(".once/triggers/activation.json"),
        &serde_json::to_vec(&activation)?,
    )?;
    tracing::info!(definition_revision = revision, generation = %activation.generation, triggers = activation.triggers.len(), "activated local trigger registration");
    Ok(json!({"activated": true, "activation": activation}))
}

pub(super) fn load(workspace: &Path) -> Result<Option<Activation>> {
    let path = workspace.join(".once/triggers/activation.json");
    let raw = match storage::read(&path) {
        Ok(raw) => raw,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let activation: Activation =
        serde_json::from_slice(&raw).context("reading trigger activation")?;
    if activation.schema != SCHEMA
        || Path::new(&activation.repository) != workspace.canonicalize()?
    {
        return Err(failure(
            "trigger_activation_mismatch",
            "trigger activation belongs to a different repository or schema",
            "Activate a trusted full definition revision in this checkout",
        ));
    }
    let diagnostics = once_frontend::validate_triggers(&activation.triggers);
    if !diagnostics.is_empty() {
        return Err(declarations_error(diagnostics));
    }
    Ok(Some(activation))
}

pub(crate) fn plan(workspace: &Path, id: &str, event: Value) -> Result<Value> {
    Ok(serde_json::to_value(select(workspace, id, event)?)?)
}

pub(super) fn select(workspace: &Path, id: &str, event: Value) -> Result<Selection> {
    storage::repository(workspace)?;
    let activation = load(workspace)?.ok_or_else(|| {
        failure(
            "trigger_not_activated",
            "no trigger definitions have been activated",
            "Run once edit activate-triggers --revision <trusted full commit object id>",
        )
    })?;
    let trigger = activation.triggers.into_iter().find(|trigger| trigger.id == id)
        .ok_or_else(|| failure("trigger_not_found", format!("activated registration has no trigger `{id}`"), "Use once query triggers to find activated ids, or explicitly activate a new definition revision"))?;
    let observation: Observation =
        serde_json::from_value(event).context("parsing ref observation")?;
    if trigger.on.event != observation.event || !trigger.on.refs.contains(&observation.reference) {
        return Err(failure(
            "trigger_event_mismatch",
            "observation does not match the activated trigger",
            format!(
                "Use event {} and one of: {}",
                trigger.on.event,
                trigger.on.refs.join(", ")
            ),
        ));
    }
    storage::revision(workspace, &observation.revision)?;
    Ok(Selection {
        schema: "once.trigger_selection.v1",
        mode: "simulation",
        contract_generation: activation.generation,
        definition_revision: activation.revision,
        observation,
        trigger,
    })
}

pub(super) fn failure(
    code: &str,
    message: impl Into<String>,
    repair: impl Into<String>,
) -> anyhow::Error {
    once_frontend::analysis::AnalysisFailure {
        diagnostic: Diagnostic::new(code, message)
            .with_attribute("trigger")
            .with_repair(repair),
    }
    .into()
}

fn declarations_error(diagnostics: Vec<Diagnostic>) -> anyhow::Error {
    // The public CLI error envelope carries one primary diagnostic.
    once_frontend::analysis::AnalysisFailure {
        diagnostic: diagnostics
            .into_iter()
            .next()
            .expect("invalid declarations have a diagnostic"),
    }
    .into()
}
