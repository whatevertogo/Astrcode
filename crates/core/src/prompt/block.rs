#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockKind {
    Identity = 0,
    Environment = 1,
    UserRules = 2,
    ProjectRules = 3,
    Skill = 4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBlock {
    pub kind: BlockKind,
    pub title: &'static str,
    pub content: String,
}
