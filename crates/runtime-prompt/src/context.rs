use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug, Default)]
pub struct PromptContext {
    pub working_dir: String,
    pub tool_names: Vec<String>,
    pub step_index: usize,
    pub turn_index: usize,
    pub vars: HashMap<String, String>,
}

impl PromptContext {
    pub fn resolve_global_var(&self, key: &str) -> Option<String> {
        match key {
            "project.working_dir" => Some(self.working_dir.clone()),
            "tools.names" => Some(self.tool_names.join(", ")),
            "run.step_index" => Some(self.step_index.to_string()),
            "run.turn_index" => Some(self.turn_index.to_string()),
            _ => self.vars.get(key).cloned(),
        }
    }

    pub fn resolve_builtin_var(&self, key: &str) -> Option<String> {
        match key {
            "env.os" => Some(std::env::consts::OS.to_string()),
            "run.date" => Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
            "run.time" => Some(chrono::Local::now().format("%H:%M:%S").to_string()),
            _ => None,
        }
    }

    pub fn contributor_cache_fingerprint(&self) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.working_dir.hash(&mut hasher);
        self.tool_names.hash(&mut hasher);

        let mut vars = self.vars.iter().collect::<Vec<_>>();
        vars.sort_by(|left, right| left.0.cmp(right.0));
        for (key, value) in vars {
            key.hash(&mut hasher);
            value.hash(&mut hasher);
        }

        format!("{:x}", hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_global_and_builtin_vars() {
        let mut ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string(), "grep".to_string()],
            step_index: 1,
            turn_index: 2,
            vars: HashMap::new(),
        };
        ctx.vars
            .insert("project.name".to_string(), "demo".to_string());

        assert_eq!(
            ctx.resolve_global_var("project.working_dir").as_deref(),
            Some("/workspace/demo")
        );
        assert_eq!(
            ctx.resolve_global_var("tools.names").as_deref(),
            Some("shell, grep")
        );
        assert_eq!(
            ctx.resolve_global_var("project.name").as_deref(),
            Some("demo")
        );
        assert_eq!(
            ctx.resolve_builtin_var("env.os").as_deref(),
            Some(std::env::consts::OS)
        );
        assert!(ctx.resolve_builtin_var("run.date").is_some());
    }
}
