## Context

插件迁移最容易犯的错误是把 discovery、loader、registry、surface assembler 全部重新塞回一个新的 runtime 中心层。当前项目已经明确不再保留这种结构，因此插件迁移必须遵守现架构：

- 发现与装配在 `server/bootstrap`
- 治理与 reload 编排在 `application`
- 全局 capability surface 在 `kernel`
- 真正的 adapter 实现继续留在 `adapter-*`

## Design Decisions

### 1. plugin 先被“物化”，再并入 surface

plugin 自身不是 `kernel` 的内部结构输入。组合根先负责：

- 发现可加载 plugin
- 装载并监督其生命周期
- 将 plugin 暴露的 hook / skill / capability 物化成统一描述

之后再交由 `kernel` 做 surface 原子替换。

### 2. reload 走治理主链，不走旁路

reload 必须通过 `application` 的治理入口触发，最终由组合根重新收集：

- builtin capabilities
- MCP capabilities
- plugin capabilities

然后把完整 surface 一次性替换进 `kernel`。不允许 plugin manager 自己悄悄刷新内部状态而 `kernel` 毫无变化。

### 3. plugin 生命周期进入治理视图

治理视图至少要表达：

- 已发现 / 已装载 plugin
- plugin 失败与不可用原因
- 当前参与 surface 的 plugin 能力结果

这样 reload 与治理快照才是有意义的。

### 4. hook 与 skill 不单独绕开能力面

plugin 暴露的 hook / skill 若要参与系统行为，必须通过明确的物化路径进入现有架构，不允许走独立的旧 runtime 注册表。

## Risks and Mitigations

### 风险：组合根再次肥大

缓解：

- 组合根只“接线”，插件发现、物化、surface 构建细节分离成独立 bootstrap 模块。

### 风险：plugin reload 成为半刷新

缓解：

- 规定 reload 后必须替换整份 capability surface，而不是只刷新 manager 内部缓存。
