## Purpose

定义 server 端 Debug-only HTTP surface，限定 debug 构建中暴露的只读 `/api/debug/*` 接口。

## Requirements

### Requirement: Debug-only HTTP surface exposes workbench APIs

server MUST 在 debug 构建中暴露 Debug Workbench 所需的只读 `/api/debug/*` 接口，并保持认证与 DTO 映射集中在 server。

#### Scenario: Debug routes are mounted in debug build

- **WHEN** server 以 debug 构建启动
- **THEN** `/api/debug/runtime/overview`、`/api/debug/runtime/timeline`、`/api/debug/sessions/{id}/trace` 与 `/api/debug/sessions/{id}/agents` 可被访问
- **AND** 它们都需要通过现有 auth 校验

#### Scenario: Debug routes are absent in non-debug build

- **WHEN** server 以 non-debug 构建启动
- **THEN** Debug Workbench 专用 `/api/debug/*` 接口不会被挂载
