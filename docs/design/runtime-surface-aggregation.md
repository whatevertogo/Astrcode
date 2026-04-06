# Runtime Surface 聚合对象设计

## 问题陈述

当前 `RuntimeSurfaceContribution` 和 `AssembledRuntimeSurface` 由多个并列的 `Vec` 字段拼接而成：

```rust
pub struct RuntimeSurfaceContribution {
    pub(crate) capability_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    pub(crate) prompt_declarations: Vec<PromptDeclaration>,
    pub(crate) skills: Vec<SkillSpec>,
    pub(crate) hook_handlers: Vec<Arc<dyn HookHandler>>,
}
```

这导致：
1. **平行传参**：bootstrap/reload 时需要在多处分别传递这些字段
2. **扩展困难**：新增字段类型需要修改多个结构体和函数签名
3. **语义不清晰**：这些字段的关联关系不明显

## 设计目标

1. **单一聚合对象**：所有 runtime surface 贡献收口到一个结构
2. **类型安全**：保留不同类型之间的语义边界
3. **易于扩展**：新增类型只需添加一个字段
4. **零拷贝传递**：使用 `Arc` 避免克隆

## 设计方案

### 1. 新的 `SurfaceLayer` 结构

按语义分层组织，而非扁平列表：

```rust
/// 运行时能力的分层聚合
///
/// 按语义将不同类型的贡献组织成清晰的层次结构，
/// 避免扁平的并列字段拼接。
#[derive(Clone, Default)]
pub struct SurfaceLayer {
    /// 能力层：工具/能力调用器
    pub capabilities: CapabilityLayer,
    /// 知识层：技能/技巧声明
    pub skills: SkillLayer,
    /// 提示词层：系统提示/指令声明
    pub prompts: PromptLayer,
    /// 钩子层：生命周期钩子
    pub hooks: HookLayer,
}

/// 能力层：可调用的工具/能力
#[derive(Clone, Default)]
pub struct CapabilityLayer {
    /// 能力调用器
    pub invokers: Vec<Arc<dyn CapabilityInvoker>>,
    /// 已注册的能力名称集合（用于冲突检测）
    pub registered_names: HashSet<String>,
}

/// 知识层：技能/技巧声明
#[derive(Clone, Default)]
pub struct SkillLayer {
    /// 技能规格
    pub specs: Vec<SkillSpec>,
    /// 技能目录（聚合后）
    pub catalog: Option<Arc<SkillCatalog>>,
}

/// 提示词层：系统提示/指令声明
#[derive(Clone, Default)]
pub struct PromptLayer {
    /// 提示词声明
    pub declarations: Vec<PromptDeclaration>,
    /// 已见过的 block_id 集合（用于去重）
    pub seen_block_ids: HashSet<String>,
}

/// 钩子层：生命周期钩子
#[derive(Clone, Default)]
pub struct HookLayer {
    /// 钩子处理器
    pub handlers: Vec<Arc<dyn HookHandler>>,
}
```

### 2. `SurfaceLayer` 的组合操作

提供方便的合并操作：

```rust
impl SurfaceLayer {
    /// 创建空层
    pub fn empty() -> Self {
        Self::default()
    }

    /// 合并多个层（后覆盖前）
    pub fn merge(layers: impl IntoIterator<Item = Self>) -> Self {
        layers.into_iter().fold(Self::empty(), |acc, layer| acc + layer)
    }

    /// 加法运算符：合并两个层
    pub fn add(self, other: Self) -> Self {
        Self {
            capabilities: self.capabilities.add(other.capabilities),
            skills: self.skills.add(other.skills),
            prompts: self.prompts.add(other.prompts),
            hooks: self.hooks.add(other.hooks),
        }
    }
}

impl Add for SurfaceLayer {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        self.add(other)
    }
}

// 各层的合并实现
impl CapabilityLayer {
    pub fn add(self, other: Self) -> Self {
        let mut registered_names = self.registered_names;
        registered_names.extend(other.registered_names);

        Self {
            invokers: {
                let mut invokers = self.invokers;
                invokers.extend(other.invokers);
                invokers
            },
            registered_names,
        }
    }
}

impl SkillLayer {
    pub fn add(self, other: Self) -> Self {
        Self {
            specs: {
                let mut specs = self.specs;
                specs.extend(other.specs);
                specs
            },
            catalog: self.catalog.or(other.catalog),
        }
    }
}

impl PromptLayer {
    pub fn add(self, other: Self) -> Self {
        let mut seen_block_ids = self.seen_block_ids;
        seen_block_ids.extend(other.seen_block_ids);

        Self {
            declarations: {
                let mut declarations = self.declarations;
                declarations.extend(other.declarations);
                declarations
            },
            seen_block_ids,
        }
    }
}

impl HookLayer {
    pub fn add(self, other: Self) -> Self {
        Self {
            handlers: {
                let mut handlers = self.handlers;
                handlers.extend(other.handlers);
                handlers
            },
        }
    }
}
```

### 3. 从贡献转换为层

```rust
impl From<RuntimeSurfaceContribution> for SurfaceLayer {
    fn from(contribution: RuntimeSurfaceContribution) -> Self {
        Self {
            capabilities: CapabilityLayer {
                invokers: contribution.capability_invokers,
                registered_names: HashSet::new(), // 稍后填充
            },
            skills: SkillLayer {
                specs: contribution.skills,
                catalog: None,
            },
            prompts: PromptLayer {
                declarations: contribution.prompt_declarations,
                seen_block_ids: HashSet::new(), // 稍后填充
            },
            hooks: HookLayer {
                handlers: contribution.hook_handlers,
            },
        }
    }
}
```

### 4. 简化后的 `AssembledRuntimeSurface`

```rust
pub(crate) struct AssembledRuntimeSurface {
    pub(crate) router: CapabilityRouter,
    pub(crate) surface: SurfaceLayer,
    pub(crate) plugin_entries: Vec<PluginEntry>,
    pub(crate) managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    pub(crate) active_plugins: Vec<ActivePluginRuntime>,
}
```

### 5. 使用示例

**之前**（平行传参）：
```rust
fn prepare_child_prompts(
    parent_prompts: Vec<PromptDeclaration>,
    profile_prompts: Vec<PromptDeclaration>,
    overrides: &ResolvedOverrides,
) -> Vec<PromptDeclaration> {
    // ...
}

fn prepare_child_hooks(
    parent_hooks: Vec<Arc<dyn HookHandler>>,
    profile_hooks: Vec<Arc<dyn HookHandler>>,
) -> Vec<Arc<dyn HookHandler>> {
    // ...
}
```

**之后**（传递聚合对象）：
```rust
fn prepare_child_surface(
    parent: &SurfaceLayer,
    profile: &AgentProfile,
    overrides: &ResolvedOverrides,
) -> SurfaceLayer {
    // 单一参数，清晰表达意图
    let mut surface = SurfaceLayer::empty();

    if overrides.inherit_prompts {
        surface.prompts = parent.prompts.clone();
    }

    if let Some(prompt) = &profile.system_prompt {
        surface.prompts.declarations.push(/* ... */);
    }

    surface
}
```

## 迁移路径

### 阶段 1：添加新结构（不破坏现有代码）
1. 添加 `SurfaceLayer` 及其子层次结构
2. 实现 `From<RuntimeSurfaceContribution> for SurfaceLayer`
3. 实现 `SurfaceLayer` 的组合操作

### 阶段 2：逐步迁移使用方
1. 迁移 `assemble_runtime_surface` 使用 `SurfaceLayer`
2. 迁移 `prepare_scoped_execution` 使用 `SurfaceLayer`
3. 迁移其他使用方

### 阶段 3：清理旧结构
1. 将 `RuntimeSurfaceContribution` 标记为 `#[deprecated]`
2. 更新文档指向新的 `SurfaceLayer`
3. 后续版本移除 `RuntimeSurfaceContribution`

## 优势

1. **清晰的语义分层**：按能力/知识/提示词/钩子组织
2. **易于扩展**：新增类型只需添加一个新层
3. **减少传参**：传递一个对象而非多个字段
4. **类型安全**：保留不同类型的边界，不混为一谈
5. **易于测试**：可以方便地构造部分层进行测试
6. **组合友好**：支持 `+` 运算符合并层

## 注意事项

1. **保持性能**：使用 `Arc` 避免大对象克隆
2. **向后兼容**：通过 `From` trait 保持兼容性
3. **渐进迁移**：不一次性重写所有代码
