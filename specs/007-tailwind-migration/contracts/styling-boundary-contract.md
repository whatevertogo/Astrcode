# Contract: 前端样式边界合同

## 目的

定义本特性完成后，前端样式在组件层、主题层和全局层之间的固定边界，作为后续实现、review 和回归检查的共同合同。

## 1. 组件层合同

- 组件静态样式必须直接在 TSX 中通过 Tailwind 类名表达
- 条件类名必须统一通过 `frontend/src/lib/utils.ts` 的 `cn(...)` 组合
- 组件不得导入 `.module.css`
- 组件不得引用 `styles.xxx`
- `style` 仅允许承载运行时动态值：
  - 动态宽高
  - 动态坐标
  - 动态 CSS 变量注入
  - 当前 Tailwind 无法稳定表达的运行时值
- 以下场景属于"Tailwind 无法稳定表达"的典型判定，仅允许在这些场景下使用 `style` 或全局 utility：
  - **运行时计算的像素值**：如拖拽定位、resize 拖拽条宽度、虚拟列表 item 高度
  - **第三方库注入的 DOM 结构**：如语法高亮、Markdown 渲染器生成的嵌套元素，无法提前知道类名
  - **CSS 变量的动态赋值**：如 `--local-x` 由 JS 计算后通过 `style` 注入，供子元素 `var()` 消费
  - **SVG 属性与 CSS 的交叉场景**：如 `stroke-dashoffset` 动画
  - **不在上述场景内的静态视觉属性（颜色、圆角、阴影、排版、间距）必须通过 Tailwind 类名表达，不得使用 `style` 回退**
- `style` 不得承载静态颜色、圆角、阴影、排版或状态样式

## 2. 主题层合同

- 颜色、阴影、圆角、间距、字体等共享视觉语义必须优先通过设计令牌暴露
- 设计令牌的底层事实源是 `frontend/src/index.css` 中的根变量
- 组件消费共享视觉语义时，应优先使用 Tailwind 主题令牌，不应复制硬编码字面量
- 新增视觉值若具有复用价值，必须先判断是否应沉淀为新令牌
- 必须维护 `sourceVar -> tailwindAlias -> usageScope -> consumer` 映射清单，且覆盖当前共享根变量

## 3. 全局层合同

`frontend/src/index.css` 只允许保留以下内容：

- reset
- 根节点布局
- 设计令牌定义
- 滚动条样式
- 共享 keyframes
- 共享 utility
- 第三方控件必要覆盖

`frontend/src/index.css` 明确禁止以下内容：

- 组件名选择器
- 组件层级结构选择器
- 局部交互状态专属规则
- 把原 CSS Module 规则整体搬运到全局层

## 4. 验收信号合同

以下信号同时成立，才视为迁移完成：

```powershell
cd D:\GitObjectsOwn\Astrcode
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cd frontend
rg --files src -g '*.module.css'
rg -n '\.module\.css|styles\.' src -g '*.ts' -g '*.tsx'
npm run typecheck
npm run build
npm run lint
npm test
```

并且：

- 按 `spec.md` 中 FR-009 的组件清单逐一做视觉对比，所有 17 个组件都有对比记录且无肉眼可见回归
- 交互状态（hover、active、focus-visible、disabled、selected、loading、error、streaming、展开/收起、上下文菜单）按组件-状态矩阵逐项验证，不允许只做每组抽样
- 桌面端滚动行为（sticky 头尾、弹窗遮罩滚动锁定、聊天区文本选择与输入焦点）保持一致
- `index.css` 审查通过，没有越界规则
- 搜索 compat 层残留（`bridge|wrapper|shim|compat|adapter.*css|module\.css`）返回 0 结果
- 设计令牌映射清单完整存在，覆盖 `frontend/src/index.css` 中当前共享颜色、阴影、布局与终端相关变量，并为每类令牌记录至少一个实际消费组件
- 主 CSS 构建产物（`dist/assets/index-*.css`）体积相较迁移前基线增长不超过 10%
