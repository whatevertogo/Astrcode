# Skills 架构

## 1. 目标

Skill 系统的目标是：

- 让模型先知道“有哪些可用技能”
- 只在命中某个 skill 时再加载正文
- 把发现、加载和执行边界拆开，避免把大量技能正文常驻系统提示

## 2. 两阶段模型

### 2.1 发现阶段

prompt 里只暴露 skill 索引：

- `name`
- `description`

模型在这一步只能知道“什么时候该用某个 skill”。

### 2.2 加载阶段

当模型决定使用 skill 时，调用内置 `Skill` 工具，再加载：

- `SKILL.md` 正文
- skill 根目录
- 可用资产文件（如 `references/`、`scripts/`）

这样做的好处是：

- 降低常驻 prompt 体积
- 避免模型被大量低相关技能正文干扰
- 保持 skill 目录可移植、可覆盖、可热重载

## 3. 目录契约

一个 skill 目录至少包含：

```text
<skill-name>/
├── SKILL.md
├── references/
└── scripts/
```

当前 `SKILL.md` frontmatter 只稳定支持：

- `name`
- `description`

其中 `name` 必须与文件夹名一致。

## 4. 加载优先级

同名 skill 的优先级为：

1. project
2. user
3. builtin

这样项目可以覆盖用户，全局用户可以覆盖内置实现。

## 5. 运行时分工

| 模块 | 职责 |
| --- | --- |
| `runtime-skill-loader` | 扫描目录、解析 `SKILL.md`、收集资产 |
| `runtime-prompt` | 生成 skill 索引块 |
| `runtime` | 提供 `Skill` 工具并负责运行时注入 |
| `core` | 承载 `SkillSpec` 等共享结构 |

## 6. 内置 skill

内置 skill 通过 build script 扫描并打包，而不是手写常量列表。

当前仓库内置 skill 仍以 `git-commit` 为主；新增 builtin skill 时，优先沿用同一目录与打包机制。

## 7. 设计边界

### 7.1 skill 不是 prompt contributor 的副产品

它既有 prompt 索引面，也有 capability surface；不能只把它看成静态文档注入。

### 7.2 skill 不是插件系统替代品

skill 更像“按需加载的工作流知识”；插件仍负责可执行能力与外部进程集成。

### 7.3 运行时元数据不写进 frontmatter

这是为了兼容 Claude 风格 skill，并保持 `SKILL.md` 可迁移、可复用。

## 8. 当前不做

- 不把所有 skill 正文预先注入系统提示
- 不在 frontmatter 里堆运行时配置
- 不让 skill 目录结构与某个传输层强绑定

## 9. 相关文档

- [./architecture.md](./architecture.md)
- [../design/README.md](../design/README.md)

