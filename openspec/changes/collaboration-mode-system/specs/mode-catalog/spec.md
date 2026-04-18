## ADDED Requirements

### Requirement: ModeCatalog trait 定义模式发现接口
系统 SHALL 在 core 中定义 `ModeCatalog` trait，包含以下方法：
- `list_modes() -> Vec<ModeSpec>`：列出所有可用模式
- `resolve_mode(id: &str) -> Option<ModeSpec>`：按 ID 查找模式

#### Scenario: ModeCatalog 可被 Arc 包装共享
- **WHEN** BuiltinModeCatalog 被 Arc<dyn ModeCatalog> 包装
- **THEN** 可跨线程安全地调用 list_modes 和 resolve_mode

### Requirement: ModeCatalog 在 bootstrap 阶段注册
系统 SHALL 在 server bootstrap 阶段创建 BuiltinModeCatalog 并将其注入到需要消费模式信息的组件中。

#### Scenario: ModeCatalog 通过 PromptFactsProvider 消费
- **WHEN** PromptFactsProvider 需要生成 ModeMap prompt block
- **THEN** 它通过 ModeCatalog trait 获取可用模式列表，不依赖具体实现
