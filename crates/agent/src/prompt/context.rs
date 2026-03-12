#[derive(Clone, Debug)]
pub struct PromptContext {
    pub working_dir: String,
    pub tool_names: Vec<String>,
    pub step_index: usize,
    pub turn_index: usize,
}
