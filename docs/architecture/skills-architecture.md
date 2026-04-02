# Skills Architecture

## Goal

AstrCode 的 skill 系统采用 Claude 风格的两阶段模型：

1. system prompt 只暴露 skill 索引（`name` + `description`）
2. 当模型命中某个 skill 时，再调用内置 `Skill` tool 按需加载正文

三个职责彻底拆开：
- `runtime-prompt` 负责发现和列出 skills
- `runtime` 负责执行 `Skill` tool、展开正文、注入运行时变量
- `SKILL.md` 只负责描述"什么时候用"和"具体怎么做"

---

## Current Flow

### Phase 1: Skill Discovery

`runtime-prompt` 在构建 prompt 时只有两件事：
- 扫描 builtin / user / project skill 目录
- 生成 skill 索引块，告诉模型有哪些 skill 可用，以及必须通过 `Skill` tool 调用

模型在这一阶段只能看到：
- skill id (= name)
- description

### Phase 2: Skill Loading

当模型决定使用 skill 时，调用内置 `Skill` tool：
- 解析当前 working directory 下实际生效的 skills
- 按 skill 名称查找目标 skill
- 返回完整 `SKILL.md` 正文
- 注入 `Base directory for this skill`
- 列出该 skill 目录下可用的资产文件

---

## Skill Directory Contract

```text
your-skill-name/
├── SKILL.md          # 正文 + frontmatter
├── references/       # 参考文档
└── scripts/          # 可执行脚本
```

### `SKILL.md` Frontmatter

当前**只认两个字段**：
```yaml
---
name: git-commit                                    # 必须与文件夹名一致 (kebab-case)
description: Use this skill when the user asks...   # 触发入口
---
```

规则：
- `name` 必须和文件夹名一致 (kebab-case)
- frontmatter 只允许 `name` 和 `description`（`deny_unknown_fields`）
- 运行时元数据不能写进 frontmatter（与外部 Claude style skill 兼容）

---

## Asset Model

除 `SKILL.md` 外的所有文件统一收集为 `asset_files`（`references/`, `scripts/`, 未来其他）。
`Skill` tool 负责暴露整个 skill 资源面。

---

## Builtin Skills

Built-in skill 由 build script 自动扫描目录，不再手写常量清单：

- **扫描脚本**: `crates/runtime/build.rs` — 编译期扫描 `crates/runtime/src/builtin_skills/*/`
- **产物**: `OUT_DIR/bundled_skills.generated.rs`（由 `include_str!` 打包所有资产）
- **运行时落盘**: `~/.astrcode/runtime/builtin-skills/<skill-name>/`（`materialize_builtin_skill_assets` 在首次启动时释放到磁盘）
- **scripts/** 目录真正落盘为文件，shell 可直接执行

当前内置 skill:
- `git-commit` (`crates/runtime/src/builtin_skills/git-commit/`, 含 `SKILL.md` + `scripts/`)

**`git-commit` skill 允许的工具**: `shell`, `readFile`, `grep`, `findFiles`, `listDir`（定义于 `bundled_skill_allowed_tools`）。

---

## Skill Loading Pipeline（运行时）

```
bootstrap_runtime()
  → builtin_skills() [crates/runtime/src/builtin_skills/mod.rs]
    → 读取编译期打包的 BUNDLED_SKILLS
    → parse_skill_md() → SkillSpec
    → bundled_skill_allowed_tools() → 注入 allowed_tools
    → materialize_builtin_skill_assets() → ~/.astrcode/runtime/builtin-skills/<id>/
  → resolve_prompt_skills(base, working_dir)
    → base (builtin) + user (~/.claude/skills + ~/.astrcode/skills) + project (<wd>/.astrcode/skills)
  → SkillTool 注册到 CapabilityRouter
```

**Skill 优先级**（`resolve_prompt_skills`）：project > user > builtin（同名覆盖）。

**用户 skill 目录**：
- `~/.claude/skills/`（Claude 兼容路径）
- `~/.astrcode/skills/`（AstrCode 专属路径，优先级更高）

**项目 skill 目录**：`<working_dir>/.astrcode/skills/`（可选）

---

## Prompt Contributors

与 skill 直接相关的 contributors (`crates/runtime-prompt/src/contributors/`):

| Contributor | 职责 |
|------------|------|
| `SkillSummaryContributor` | 产出 skill 索引（name + description），仅当工具列表含 `Skill` 时激活 |
| `WorkflowExamplesContributor` | few-shot 行为约束（首步示例） |
| `IdentityContributor` | 助手身份声明（加载 `~/.astrcode/IDENTITY.md`） |
| `AgentsMdContributor` | `AGENTS.md` 加载（根目录 + 子目录逐层查找） |
| `EnvironmentContributor` | 环境上下文（OS、shell、cwd） |
| `CapabilityPromptContributor` | 工具描述注入（从 `CapabilityDescriptor` 提取） |

**完整 contributor 列表**（`PromptComposer::with_defaults()` 顺序）：
1. `IdentityContributor` (block kind=100)
2. `EnvironmentContributor` (block kind=200)
3. `AgentsMdContributor` (block kind=300)
4. `CapabilityPromptContributor` (block kind=400)
5. `SkillSummaryContributor` (block kind=600)
6. `WorkflowExamplesContributor` (block kind=700)

---

## Prompt Composition Engine

`PromptComposer` (`crates/runtime-prompt/src/composer.rs`):
- 按依赖关系拓扑排序 contributor（wave-based topological sort）
- 支持条件渲染：`Always`, `StepEquals`, `FirstStepOnly`, `HasTool`, `VarEquals`
- 支持渲染目标：`System`, `PrependUser`, `PrependAssistant`, `AppendUser`, `AppendAssistant`
- 缓存策略：per-contributor cache fingerprint + TTL
- 诊断系统：`PromptDiagnostics`（`Off` / `Warn` / `Strict`）

`SkillSpec` 结构：
```rust
pub struct SkillSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    pub guide: String,
    pub skill_root: Option<String>,
    pub asset_files: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub source: SkillSource,  // Builtin | User | Project | Plugin | Mcp
}
```

---

## Capability Surface

`Skill` 是 capability surface 的一部分（不是 prompt contributor 的副产品）：
- `SKILL_TOOL_NAME = "Skill"` — 工具名固定
- skill 加载通过统一的 capability router 暴露
- runtime reload 时，`Skill` tool 跟随 builtin capability surface 一起重建

---

## Key Files

| 文件 | 职责 |
|------|------|
| `crates/runtime-prompt/src/skill_loader.rs` | 目录扫描、`parse_skill_md`、`collect_asset_files`、`load_user_skills`、`load_project_skills`、`resolve_prompt_skills` |
| `crates/runtime-prompt/src/skill_spec.rs` | `SkillSpec`, `SkillSource`, `is_valid_skill_name`, `normalize_skill_name` |
| `crates/runtime-prompt/src/contributors/skill_summary.rs` | Skill 索引 contributor |
| `crates/runtime/src/skill_tool.rs` | `Skill` tool 实现（`Tool` trait） |
| `crates/runtime/src/builtin_capabilities.rs` | Capability 装配 (含 Skill) |
| `crates/runtime/src/builtin_skills/` | Builtin skill 目录 (`git-commit/`) |
| `crates/runtime/build.rs` | 编译期自动扫描 builtin skills, 生成 `bundled_skills.generated.rs` |
| `crates/runtime/src/builtin_skills/mod.rs` | 运行时加载 builtin skills（`include!` 生成文件 + 资源落盘） |
