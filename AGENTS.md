# Repository Guidelines

本项目不需要向后兼容的代码，只需要良好代码架构和干净代码

## 注意

- 用中文注释，且注释尽量表明为什么和做了什么、
- 为了干净架构和良好实现可以不需要向后兼容，如果向后兼容需要说明为什么
- 最后需要cargo fmt --all --check  && cargo clippy --all-targets --all-features -- -D warnings && cargo test验证你的更改
- 前端css不允许出现webview相关内容这会导致应用端无法下滑窗口
- 你必须用中文写文档
