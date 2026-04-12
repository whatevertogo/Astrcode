#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComposerOptionKind {
    Command,
    Skill,
    Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOptionsRequest {
    pub query: Option<String>,
    pub kinds: Vec<ComposerOptionKind>,
    pub limit: usize,
}

impl Default for ComposerOptionsRequest {
    fn default() -> Self {
        Self {
            query: None,
            kinds: Vec::new(),
            limit: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOption {
    pub kind: ComposerOptionKind,
    pub id: String,
    pub title: String,
    pub description: String,
    pub insert_text: String,
    pub badges: Vec<String>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ComposerService;

impl ComposerService {
    pub fn list_options(&self, request: ComposerOptionsRequest) -> Vec<ComposerOption> {
        let mut items = vec![
            ComposerOption {
                kind: ComposerOptionKind::Command,
                id: "compact".to_string(),
                title: "压缩上下文".to_string(),
                description: "压缩当前会话上下文".to_string(),
                insert_text: "/compact".to_string(),
                badges: vec!["built-in".to_string()],
                keywords: vec!["compact".to_string(), "compress".to_string()],
            },
            ComposerOption {
                kind: ComposerOptionKind::Skill,
                id: "git-commit".to_string(),
                title: "Git 提交".to_string(),
                description: "生成并执行规范提交".to_string(),
                insert_text: "/git-commit".to_string(),
                badges: vec!["skill".to_string()],
                keywords: vec!["git".to_string(), "commit".to_string()],
            },
            ComposerOption {
                kind: ComposerOptionKind::Capability,
                id: "readFile".to_string(),
                title: "读取文件".to_string(),
                description: "读取工作区文件内容".to_string(),
                insert_text: "readFile".to_string(),
                badges: vec!["capability".to_string()],
                keywords: vec!["read".to_string(), "file".to_string()],
            },
        ];

        if !request.kinds.is_empty() {
            items.retain(|item| request.kinds.contains(&item.kind));
        }

        if let Some(query) = request.query {
            let query = query.to_lowercase();
            items.retain(|item| {
                item.id.to_lowercase().contains(&query)
                    || item.title.to_lowercase().contains(&query)
                    || item.description.to_lowercase().contains(&query)
                    || item
                        .keywords
                        .iter()
                        .any(|keyword| keyword.to_lowercase().contains(&query))
            });
        }

        if items.len() > request.limit {
            items.truncate(request.limit);
        }
        items
    }
}
