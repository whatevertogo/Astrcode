use astrcode_governance_contract::GovernanceModeSpec;

use crate::PluginDescriptor;

pub fn builtin_modes_descriptor(
    plugin_id: impl Into<String>,
    display_name: impl Into<String>,
    modes: Vec<GovernanceModeSpec>,
) -> PluginDescriptor {
    let mut descriptor = PluginDescriptor::builtin(plugin_id, display_name);
    descriptor.modes = modes;
    descriptor
}
