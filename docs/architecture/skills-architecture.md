# Skills Architecture

## Goal

AstrCode 的 skill 系统现在采用 Claude 风格的两阶段模型：

1. `system prompt` 只暴露 skill 索引
2. 当模型命中某个 skill 时，再调用内置 `Skill` tool 按需加载正文

这样做的目的不是“模仿格式”，而是把三个职责彻底拆开：

- `runtime-prompt` 负责发现和列出 skills
- `runtime` 负责执行 `Skill` tool、展开正文、注入运行时变量
- `SKILL.md` 只负责描述“什么时候用”和“具体怎么做”

这比旧的一阶段实现更清晰，因为旧实现会在 prompt 组装时直接把匹配到的整份 skill 正文塞进系统提示，导致：

- prompt 体积不稳定
- skill 匹配和 skill 加载耦合在一起
- builtin / user / project skill 很难走统一链路
- `scripts/`、`references/` 这类资源虽然存在，但没有一个明确的按需加载入口

---

## Current Flow

### Phase 1: Skill Discovery

`runtime-prompt` 在构建 prompt 时只做两件事：

- 扫描 builtin / user / project skill 目录
- 生成一个 skill 索引块，告诉模型有哪些 skill 可用，以及必须通过 `Skill` tool 调用它们

涉及文件：

- `crates/runtime-prompt/src/skill_loader.rs`
- `crates/runtime-prompt/src/skill_spec.rs`
- `crates/runtime-prompt/src/contributors/skill_summary.rs`

模型在这一阶段能看到的核心信息只有：

- `skill id`
- `description`

这和 Claude Code 的思路一致：`description` 是触发入口，完整正文不应该默认进入 prompt。

### Phase 2: Skill Loading

当模型决定使用 skill 时，会调用内置 `Skill` tool。

`Skill` tool 会：

- 解析当前 working directory 下实际生效的 skills
- 按 skill 名称查找目标 skill
- 返回完整 `SKILL.md` 正文
- 注入 `Base directory for this skill`
- 展开 `${CLAUDE_SKILL_DIR}` / `${ASTRCODE_SKILL_DIR}`
- 展开 `${CLAUDE_SESSION_ID}` / `${ASTRCODE_SESSION_ID}`
- 列出该 skill 目录下可用的资产文件

涉及文件：

- `crates/runtime/src/skill_tool.rs`
- `crates/runtime/src/builtin_capabilities.rs`
- `crates/runtime/src/runtime_surface_assembler.rs`

---

## Skill Directory Contract

skill 目录格式固定为：

```text
your-skill-name/
├── SKILL.md
└── references/
    └── ...
```

也允许存在：

```text
your-skill-name/
├── SKILL.md
├── references/
└── scripts/
```

AstrCode 现在把整个 skill 文件夹都视为资源面，而不是只认 `SKILL.md`。

### `SKILL.md` Frontmatter

当前只认两个字段：

```yaml
---
name: git-commit
description: Use this skill when the user asks you to prepare and run a git commit. Do NOT use for general git explanations.
---
```

规则：

- `name` 必须和文件夹名一致
- `name` 必须是 kebab-case
- frontmatter 只允许 `name` 和 `description`
- 运行时元数据不能再写进 `SKILL.md` frontmatter`

这样做是为了让 skill markdown 和运行时实现解耦，方便迁移外部 Claude 风格 skill 仓库。

---

## Asset Model

`skill_loader` 会把除 `SKILL.md` 外的所有文件统一收集为 `asset_files`。

这意味着下面这些目录都会进入索引：

- `references/`
- `scripts/`
- 未来新增的其他辅助文件夹

为什么统一成 `asset_files`，而不是只保留 `reference_files`：

- Claude 风格 skill 不只有参考文档，还有可执行脚本
- runtime 不应该预设“只有 references 才重要”
- `Skill` tool 的职责是暴露整个 skill 资源面，而不是替模型做资源分类判断

---

## Builtin Skills

builtin skill 不再手写 Rust 常量列表维护资源，而是由 build script 自动扫描目录：

- `crates/runtime/build.rs`
- `crates/runtime/src/builtin_skills/mod.rs`

流程：

1. build script 扫描 `crates/runtime/src/builtin_skills/*/`
2. 找到包含 `SKILL.md` 的文件夹
3. 把整目录资源生成到 `bundled_skills.generated.rs`
4. runtime 启动时把这些资源物化到：

```text
~/.astrcode/runtime/builtin-skills/<skill-name>/
```

这样做的意义：

- builtin skill 与 user/project skill 拥有同样的目录结构
- `scripts/` 不再只是编译期字符串，而是真正落盘的文件
- shell 可以直接执行 builtin skill 自带脚本

---

## Prompt Contributors

当前默认 prompt contributors 中，和 skill 直接相关的是：

- `SkillSummaryContributor`
- `WorkflowExamplesContributor`

不再存在旧的 `SkillGuideContributor`。

职责划分：

- `SkillSummaryContributor`：产出 skill 索引
- `WorkflowExamplesContributor`：保留首步 few-shot 行为约束

这样命名更准确，因为旧的 `SkillSummaryContributor` 实际上承载的是 few-shot 示例，不是 skill summary。

---

## Capability Surface

`Skill` 现在是 capability surface 的一部分，而不是 prompt contributor 的副产品。

也就是说：

- skill 加载通过统一的 capability router 暴露
- plugin capability 和 builtin capability 共享同一注册入口
- runtime reload 时，`Skill` tool 会跟随 builtin capability surface 一起重建

这符合仓库当前的统一能力路由原则，避免再出现“skills 走一套半隐藏机制，tools 走另一套正式协议”的分裂。

---

## Tradeoffs

### Why This Is Better Than The Old Flow

- skill 匹配和 skill 加载分层更清楚
- prompt 大小更稳定
- builtin / user / project skill 统一建模
- `scripts/` 和 `references/` 都能被正确索引和暴露
- 更接近 Claude Code 的心智模型，迁移成本更低

### Known Boundary

AstrCode 当前只对 Claude 风格 skill 的“两阶段加载”和“目录资源模型”做了对齐，没有复制 Claude 的整套 command 系统，例如：

- prompt command registry
- `context: fork` 那种子 agent 执行语义
- `disable-model-invocation`
- 插件 marketplace 级 skill 遥测

这些属于更高层的 command runtime，不属于当前 skill 架构收口的目标范围。

---

## Key Files

- `crates/runtime-prompt/src/skill_loader.rs`
- `crates/runtime-prompt/src/skill_spec.rs`
- `crates/runtime-prompt/src/contributors/skill_summary.rs`
- `crates/runtime-prompt/src/contributors/workflow_examples.rs`
- `crates/runtime/src/skill_tool.rs`
- `crates/runtime/src/builtin_capabilities.rs`
- `crates/runtime/src/builtin_skills/mod.rs`
- `crates/runtime/build.rs`
