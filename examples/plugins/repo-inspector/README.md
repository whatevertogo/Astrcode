# Repo Inspector Example Plugin

`repo-inspector` 是一个真实的 AstrCode V4 示例插件，不是测试专用 fixture。

它通过 `cargo run -p astrcode-example-plugin --quiet` 启动，并暴露两个 `coding` profile 下的 capability：

- `workspace.summary`
- `file.preview`

## 目录说明

- `plugin.toml`: 平台发现与启动该插件时使用的 manifest
- `examples/example-plugin`: 插件实际实现

## 本地接入

把插件目录加入 `ASTRCODE_PLUGIN_DIRS` 后启动 server。
这个环境变量已收口到 `crates/runtime-config/src/constants.rs` 的 plugin 分类：

```powershell
$env:ASTRCODE_PLUGIN_DIRS = (Resolve-Path "examples/plugins/repo-inspector")
cargo run -p astrcode-server
```

平台启动后会：

1. 发现 `plugin.toml`
2. 启动 `astrcode-example-plugin`
3. 通过 V4 `initialize` 握手获取 capability
4. 把插件 capability 和内置 tool 一起接入统一 `CapabilityRouter`

## 设计目的

这个示例用于演示：

- `stdio` transport
- `Peer / Supervisor / Worker`
- `coding profile` 上下文读取
- 插件 capability 进入平台统一 capability 路由
