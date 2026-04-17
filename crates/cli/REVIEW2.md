# CLI Crate 第二轮审查：视觉与最佳实践

审查范围：`crates/cli/src/` 全部文件（修复后的最新版本）
审查日期：2026-04-17

---

## Visual - 可访问性

### 1. ascii_only 模式下 marker 符号碰撞，不同 cell 类型无法区分
- **文件**: `ui/cells.rs:342-363`
- **描述**: ascii fallback 中 `assistant_marker` 和 `thinking_marker` 都返回 `"*"`，`tool_marker` 和 `secondary_marker` 都返回 `"-"`。无颜色终端下，用户看到 `*` 无法区分 assistant 输出和 thinking，看到 `-` 无法区分 tool call 和系统提示。
- **建议**: 为每种类型分配唯一的 ascii 符号，如 assistant `*`、thinking `~`、tool `>`、secondary `-`、tool_block `|`。

### 2. hero card `two_col_row` 中 `│` 硬编码，ascii_only 模式下会显示错误
- **文件**: `ui/hero.rs:243`
- **代码**: `format!("{}│{}", ...)`
- **描述**: `framed_rows` 中正确使用了 `theme.glyph("│", "|")`，但 `two_col_row` 内部的列分隔符直接硬编码了 Unicode `│`，在 ascii_only 终端下会显示为乱码或空格。
- **建议**: 将 theme 传入 `two_col_row` 或提取 `│` 的 glyph 选择。

### 3. banner 文案混合中英文术语
- **文件**: `ui/transcript.rs:32`
- **代码**: `"stream 需要重新同步，继续操作前建议等待恢复。"`
- **描述**: 用户可见的错误提示中混用了英文 "stream" 和中文描述，与 thinking.rs 中已修复的统一语言策略不一致。
- **建议**: 统一为纯中文，如"流需要重新同步..."。

### 4. hero 默认标题 `Astrcode workspace` 是英文
- **文件**: `ui/hero.rs:19`
- **代码**: `.unwrap_or("Astrcode workspace")`
- **描述**: 其他所有面向用户的文案（提示、状态、footer hint）都是中文，唯独这个 fallback 标题是英文。
- **建议**: 统一为中文，如 "Astrcode 工作区"。

---

## Visual - 一致性

### 5. `format!("{phase:?}")` 用于用户可见的 phase 标签
- **文件**: `ui/hero.rs:29`, `ui/footer.rs:99`
- **描述**: 两处通过 Debug 格式化 `AstrcodePhaseDto` 生成用户可见文本（如 "streaming"、"idle"）。Debug 输出不是 UI 文案，枚举重命名会导致用户看到的变化。且 hero 和 footer 中的 phase 用途相同但来源独立构造。
- **建议**: 为 `AstrcodePhaseDto` 添加 `Display` 实现或专用的 `display_label()` 方法。

### 6. footer 实际只有 3 行内容，但 `FOOTER_HEIGHT = 5`
- **文件**: `render/mod.rs:12,68-110`
- **描述**: `footer_lines` 返回 3 行（status、input、hint），`render_footer` 渲染 5 行布局（3 内容 + 2 divider）。虽然功能正确，但 `FOOTER_HEIGHT` 的语义不清晰——它代表的是布局高度而非内容行数，且 footer_lines 的 3 行数量与 render_footer 的 5 行布局之间没有编译时保证。如果 footer_lines 返回的行数变化，render_footer 会 panic（`footer.lines[0]` 索引越界）。
- **建议**: 定义 `const FOOTER_CONTENT_LINES: usize = 3`，并在 render_footer 中用常量而非硬编码索引。或者让 footer_lines 自身返回包含 divider 的完整 5 行。

### 7. hero 提示文案与 footer hint 中的快捷键描述不一致
- **文件**: `ui/hero.rs:96,102,110` vs `ui/footer.rs:89,109`
- **描述**: hero 中写"输入 / 打开 commands"，footer 中写"/ commands"。hero 中写"Tab 在 transcript / composer 间切换"，footer 中写"Tab 切换焦点"。同一个操作在不同位置有不同描述。
- **建议**: 统一两处的快捷键描述文案。

---

## Performance

### 8. `selected_transcript_cell()` 每次调用都全量投影 transcript
- **文件**: `state/mod.rs:209-213`
- **描述**: `selected_transcript_cell()` 调用 `transcript_cells()` 投影整个 transcript，然后取第 N 个。这个方法被 `toggle_selected_cell_expanded` 和 `selected_cell_is_thinking` 调用，每次操作都是 O(n) 全量投影。
- **建议**: 添加 `project_single_cell(&self, index: usize)` 方法，只投影一个 cell；或在 `transcript_cells()` 上层缓存结果。

### 9. `should_animate_thinking_playback` 遍历 transcript 两次
- **文件**: `state/mod.rs:345-387`
- **描述**: 先遍历所有 cells 检查 streaming thinking（345-355），然后再遍历一次检查 synthetic 条件（372-387）。两次遍历可以合并为一次。
- **建议**: 合并为单次遍历，先记录是否已有 streaming thinking/assistant/tool，再决定是否需要 animation。

### 10. `apply_stream_envelope` 每次流式 delta 都 clone `slash_candidates`
- **文件**: `state/mod.rs:309`
- **描述**: `self.conversation.slash_candidates.clone()` 在每次 envelope 到达时执行。slash_candidates 列表通常不频繁变化，但每个流式 chunk 都会触发一次完整 clone。
- **建议**: 仅在 slash_candidates 实际变化时（`ReplaceSlashCandidates` delta）才调用 `sync_slash_items`。

### 11. `visible_input_state` 中不必要的 `visible_before.clone()`
- **文件**: `ui/footer.rs:153`
- **描述**: `let mut visible = visible_before.clone()` 后，`visible_before` 不再使用。可以直接 move 而非 clone。
- **建议**: 改为 `let mut visible = visible_before;`。

---

## Best Practices

### 12. `enum_wire_name` 仍通过 serde 序列化判断 stdout/stderr
- **文件**: `state/conversation.rs:244-252`
- **描述**: 虽然比之前的 `format!("{:?}", stream)` 好一些，但仍然通过 `serde_json::to_value` 序列化枚举后取字符串值来判断变体。如果 `ToolOutputStream` 是外部 crate 类型且不暴露变体，这是唯一的方式，但应该有注释说明为什么不能直接 match。
- **建议**: 添加注释说明限制原因，或者在外部 crate 中为 `ToolOutputStream` 添加 `is_stderr()` 方法。

### 13. `render_transcript` 中 `Paragraph::new().wrap()` 可能二次换行
- **文件**: `render/mod.rs:54-65`
- **描述**: `wrap_text` 已经手动将文本按列宽换行，然后 `Paragraph::new().wrap(Wrap { trim: false })` 又启用了 ratatui 的自动换行。虽然 scroll 机制依赖 Paragraph，但 wrap 是多余的，可能在边界条件下导致意外行为。
- **建议**: 使用 `Paragraph::new().wrap(Wrap { trim: false })` 是必要的（因为 scroll 依赖 Paragraph 内部换行），但可以验证两者的一致性，或移除手动 wrap 改为完全依赖 ratatui 的 wrap。

### 14. `palette_next` / `palette_prev` 仍有 Resume/Slash 两个重复分支
- **文件**: `state/interaction.rs:413-437`
- **描述**: 两个方法中 Resume 和 Slash 分支的逻辑完全相同（只是 +1 / -1 的区别）。可以提取为辅助方法。
- **建议**: 提取 `fn advance_selected(items_len: usize, selected: &mut usize, forward: bool)` 辅助函数。

### 15. `SharedStreamPacer` 中 `expect("stream pacer lock poisoned")` 出现 5 次
- **文件**: `app/mod.rs:212, 223, 231, 245, 249`
- **描述**: 每个 lock 操作都有相同的 `expect` 字符串。如果某处 panic，其余地方也会连锁 panic。虽然 lock poisoning 在正常使用中不应发生，但可以统一处理。
- **建议**: 提取 `fn lock_inner(&self) -> std::sync::MutexGuard<'_, StreamPacerState>` 方法，统一处理 lock poisoning。

---

## 汇总

| 类别 | 数量 | 核心问题 |
|------|------|----------|
| 可访问性 | 4 | ascii marker 碰撞、硬编码 Unicode 字符、中英混用 |
| 视觉一致性 | 3 | Debug 格式化做 UI 文案、footer 布局/内容耦合、快捷键描述不一致 |
| 性能 | 4 | 全量投影、双次遍历、多余 clone、不必要的 clone |
| 最佳实践 | 4 | serde 判枚举、双重 wrap、重复分支、lock poisoning 处理 |

### 优先修复建议

1. **第 2 项**（`│` 硬编码）— ascii_only 模式下直接显示错误
2. **第 1 项**（ascii marker 碰撞）— 影响无颜色终端用户的基本可用性
3. **第 10 项**（slash_candidates 多余 clone）— 流式场景下每个 delta 触发，性能影响最大
4. **第 8 项**（全量投影）— 每次键盘操作触发 O(n) 投影
