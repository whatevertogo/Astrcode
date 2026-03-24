#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioTransportConfig {
    pub line_delimited_json: bool,
}

impl Default for StdioTransportConfig {
    fn default() -> Self {
        Self {
            line_delimited_json: true,
        }
    }
}
