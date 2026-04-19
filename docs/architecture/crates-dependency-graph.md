# Crates Dependency Graph

自动生成文件，请勿手工编辑。

- 生成命令：`node scripts/generate-crate-deps-graph.mjs`

## Mermaid

```mermaid
graph TD
  astrcode-adapter-agents[astrcode-adapter-agents] --> astrcode-core[astrcode-core]
  astrcode-adapter-llm[astrcode-adapter-llm] --> astrcode-core[astrcode-core]
  astrcode-adapter-mcp[astrcode-adapter-mcp] --> astrcode-adapter-prompt[astrcode-adapter-prompt]
  astrcode-adapter-mcp[astrcode-adapter-mcp] --> astrcode-core[astrcode-core]
  astrcode-adapter-prompt[astrcode-adapter-prompt] --> astrcode-core[astrcode-core]
  astrcode-adapter-skills[astrcode-adapter-skills] --> astrcode-core[astrcode-core]
  astrcode-adapter-storage[astrcode-adapter-storage] --> astrcode-core[astrcode-core]
  astrcode-adapter-tools[astrcode-adapter-tools] --> astrcode-core[astrcode-core]
  astrcode-application[astrcode-application] --> astrcode-core[astrcode-core]
  astrcode-application[astrcode-application] --> astrcode-kernel[astrcode-kernel]
  astrcode-application[astrcode-application] --> astrcode-session-runtime[astrcode-session-runtime]
  astrcode-cli[astrcode-cli] --> astrcode-client[astrcode-client]
  astrcode-cli[astrcode-cli] --> astrcode-core[astrcode-core]
  astrcode-client[astrcode-client] --> astrcode-protocol[astrcode-protocol]
  astrcode-core[astrcode-core]
  astrcode-kernel[astrcode-kernel] --> astrcode-core[astrcode-core]
  astrcode-plugin[astrcode-plugin] --> astrcode-core[astrcode-core]
  astrcode-plugin[astrcode-plugin] --> astrcode-protocol[astrcode-protocol]
  astrcode-protocol[astrcode-protocol] --> astrcode-core[astrcode-core]
  astrcode-sdk[astrcode-sdk] --> astrcode-core[astrcode-core]
  astrcode-sdk[astrcode-sdk] --> astrcode-protocol[astrcode-protocol]
  astrcode-server[astrcode-server] --> astrcode-adapter-agents[astrcode-adapter-agents]
  astrcode-server[astrcode-server] --> astrcode-adapter-llm[astrcode-adapter-llm]
  astrcode-server[astrcode-server] --> astrcode-adapter-mcp[astrcode-adapter-mcp]
  astrcode-server[astrcode-server] --> astrcode-adapter-prompt[astrcode-adapter-prompt]
  astrcode-server[astrcode-server] --> astrcode-adapter-skills[astrcode-adapter-skills]
  astrcode-server[astrcode-server] --> astrcode-adapter-storage[astrcode-adapter-storage]
  astrcode-server[astrcode-server] --> astrcode-adapter-tools[astrcode-adapter-tools]
  astrcode-server[astrcode-server] --> astrcode-application[astrcode-application]
  astrcode-server[astrcode-server] --> astrcode-core[astrcode-core]
  astrcode-server[astrcode-server] --> astrcode-kernel[astrcode-kernel]
  astrcode-server[astrcode-server] --> astrcode-plugin[astrcode-plugin]
  astrcode-server[astrcode-server] --> astrcode-protocol[astrcode-protocol]
  astrcode-server[astrcode-server] --> astrcode-session-runtime[astrcode-session-runtime]
  astrcode-session-runtime[astrcode-session-runtime] --> astrcode-core[astrcode-core]
  astrcode-session-runtime[astrcode-session-runtime] --> astrcode-kernel[astrcode-kernel]
```

## Crate 依赖表

| Crate | Path | Internal Deps Count | Internal Deps |
|---|---|---:|---|
| astrcode-adapter-agents | crates/adapter-agents | 1 | astrcode-core |
| astrcode-adapter-llm | crates/adapter-llm | 1 | astrcode-core |
| astrcode-adapter-mcp | crates/adapter-mcp | 2 | astrcode-adapter-prompt, astrcode-core |
| astrcode-adapter-prompt | crates/adapter-prompt | 1 | astrcode-core |
| astrcode-adapter-skills | crates/adapter-skills | 1 | astrcode-core |
| astrcode-adapter-storage | crates/adapter-storage | 1 | astrcode-core |
| astrcode-adapter-tools | crates/adapter-tools | 1 | astrcode-core |
| astrcode-application | crates/application | 3 | astrcode-core, astrcode-kernel, astrcode-session-runtime |
| astrcode-cli | crates/cli | 2 | astrcode-client, astrcode-core |
| astrcode-client | crates/client | 1 | astrcode-protocol |
| astrcode-core | crates/core | 0 | - |
| astrcode-kernel | crates/kernel | 1 | astrcode-core |
| astrcode-plugin | crates/plugin | 2 | astrcode-core, astrcode-protocol |
| astrcode-protocol | crates/protocol | 1 | astrcode-core |
| astrcode-sdk | crates/sdk | 2 | astrcode-core, astrcode-protocol |
| astrcode-server | crates/server | 13 | astrcode-adapter-agents, astrcode-adapter-llm, astrcode-adapter-mcp, astrcode-adapter-prompt, astrcode-adapter-skills, astrcode-adapter-storage, astrcode-adapter-tools, astrcode-application, astrcode-core, astrcode-kernel, astrcode-plugin, astrcode-protocol, astrcode-session-runtime |
| astrcode-session-runtime | crates/session-runtime | 2 | astrcode-core, astrcode-kernel |
