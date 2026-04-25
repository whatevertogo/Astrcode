//! 内置 mode prompt 资源。
//!
//! 计划模式的主流程和模板拆成 markdown 资源文件，便于后续直接微调文本，
//! 避免继续把完整提示词硬编码在 Rust 字符串里。

pub(crate) fn code_mode_prompt() -> &'static str {
    "You are in code mode. Prefer thinking and then direct progress, make concrete code changes \
     when needed, and use delegation only when isolation or parallelism materially helps. If the \
     user explicitly wants a plan or the task clearly needs up-front planning, call \
     `enterPlanMode` first instead of mixing planning into execution."
}

pub(crate) fn plan_mode_prompt() -> &'static str {
    include_str!("builtin_prompts/plan_mode.md")
}

pub(crate) fn plan_mode_reentry_prompt() -> &'static str {
    include_str!("builtin_prompts/plan_mode_reentry.md")
}

pub(crate) fn plan_mode_exit_prompt() -> &'static str {
    include_str!("builtin_prompts/plan_mode_exit.md")
}

pub(crate) fn plan_template_prompt() -> &'static str {
    include_str!("builtin_prompts/plan_template.md")
}
