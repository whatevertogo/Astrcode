use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use astrcode_core::{
    AstrError, Result,
    mode::{BoundModeToolContractSnapshot, GovernanceModeSpec, ModeId},
};

use crate::mode::validate_mode_transition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerModeSummary {
    pub id: ModeId,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ServerModeCatalogEntry {
    pub spec: GovernanceModeSpec,
    pub builtin: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ServerModeCatalogSnapshot {
    pub entries: BTreeMap<String, ServerModeCatalogEntry>,
}

impl ServerModeCatalogSnapshot {
    pub(crate) fn get(&self, mode_id: &ModeId) -> Option<&GovernanceModeSpec> {
        self.entries.get(mode_id.as_str()).map(|entry| &entry.spec)
    }

    pub(crate) fn list(&self) -> Vec<ServerModeSummary> {
        self.entries
            .values()
            .map(|entry| ServerModeSummary {
                id: entry.spec.id.clone(),
                name: entry.spec.name.clone(),
                description: entry.spec.description.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ServerModeCatalog {
    snapshot: Arc<RwLock<ServerModeCatalogSnapshot>>,
}

impl ServerModeCatalog {
    pub(crate) fn from_mode_specs(
        builtin_modes: Vec<GovernanceModeSpec>,
        plugin_modes: Vec<GovernanceModeSpec>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self::new(build_snapshot(
            builtin_modes,
            plugin_modes,
        )?)))
    }

    pub(crate) fn new(snapshot: ServerModeCatalogSnapshot) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(snapshot)),
        }
    }

    pub(crate) fn snapshot(&self) -> ServerModeCatalogSnapshot {
        self.snapshot
            .read()
            .expect("server mode catalog lock poisoned")
            .clone()
    }

    pub(crate) fn list(&self) -> Vec<ServerModeSummary> {
        self.snapshot().list()
    }

    pub(crate) fn get(&self, mode_id: &ModeId) -> Option<GovernanceModeSpec> {
        self.snapshot().get(mode_id).cloned()
    }

    pub(crate) fn preview_plugin_modes(
        &self,
        plugin_modes: Vec<GovernanceModeSpec>,
    ) -> Result<ServerModeCatalogSnapshot> {
        let current = self.snapshot();
        let builtin_modes = current
            .entries
            .values()
            .filter(|entry| entry.builtin)
            .map(|entry| entry.spec.clone())
            .collect::<Vec<_>>();
        build_snapshot(builtin_modes, plugin_modes)
    }

    pub(crate) fn replace_snapshot(&self, snapshot: ServerModeCatalogSnapshot) {
        *self
            .snapshot
            .write()
            .expect("server mode catalog lock poisoned") = snapshot;
    }

    pub(crate) fn validate_transition(
        &self,
        from_mode_id: &ModeId,
        to_mode_id: &ModeId,
    ) -> Result<()> {
        validate_mode_transition(self, from_mode_id, to_mode_id)?;
        Ok(())
    }

    pub(crate) fn bound_tool_contract_snapshot(
        &self,
        mode_id: &ModeId,
    ) -> Result<BoundModeToolContractSnapshot> {
        let snapshot = self.snapshot();
        let entry = snapshot
            .entries
            .get(mode_id.as_str())
            .ok_or_else(|| AstrError::Validation(format!("unknown mode '{}'", mode_id)))?;
        Ok(BoundModeToolContractSnapshot {
            mode_id: entry.spec.id.clone(),
            artifact: entry.spec.artifact.clone(),
            exit_gate: entry.spec.exit_gate.clone(),
        })
    }
}

fn build_snapshot(
    builtin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
    plugin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
) -> Result<ServerModeCatalogSnapshot> {
    let mut entries = BTreeMap::new();
    for (builtin, spec) in builtin_modes
        .into_iter()
        .map(|spec| (true, spec))
        .chain(plugin_modes.into_iter().map(|spec| (false, spec)))
    {
        spec.validate()?;
        let mode_id = spec.id.as_str().to_string();
        if entries.contains_key(&mode_id) {
            return Err(AstrError::Validation(format!(
                "duplicate mode id '{}'",
                mode_id
            )));
        }
        entries.insert(mode_id, ServerModeCatalogEntry { spec, builtin });
    }
    Ok(ServerModeCatalogSnapshot { entries })
}
