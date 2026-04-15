use std::path::PathBuf;

use crate::capability::TerminalCapabilities;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellState {
    pub connection_origin: String,
    pub working_dir: Option<PathBuf>,
    pub capabilities: TerminalCapabilities,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            connection_origin: String::new(),
            working_dir: None,
            capabilities: TerminalCapabilities::detect(),
        }
    }
}

impl ShellState {
    pub fn new(
        connection_origin: String,
        working_dir: Option<PathBuf>,
        capabilities: TerminalCapabilities,
    ) -> Self {
        Self {
            connection_origin,
            working_dir,
            capabilities,
        }
    }
}
