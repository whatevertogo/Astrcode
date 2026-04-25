## ADDED Requirements

### Requirement: runtime hook points SHALL use typed event contexts
`agent-runtime` MUST build event-specific hook contexts instead of sending generic metadata for all events.

#### Scenario: tool call hook receives tool context
- **WHEN** runtime dispatches `tool_call`
- **THEN** payload SHALL include concrete tool id, tool name, parsed args, capability spec, step index, and turn identity

### Requirement: tool_call denial SHALL produce a tool result, not a turn error
When `tool_call` hook returns per-tool denial, runtime MUST record it as failed tool result for that call.

#### Scenario: one parallel tool is denied
- **WHEN** a batch contains multiple tool calls and one is denied
- **THEN** runtime SHALL skip only the denied tool
- **AND** allowed calls MAY execute normally

### Requirement: tool_result hooks SHALL run before recording tool results
`agent-runtime` MUST dispatch `tool_result` before emitting durable tool result events or appending result messages.

#### Scenario: hook redacts result
- **WHEN** a hook returns a redacted tool result
- **THEN** only the redacted result SHALL be recorded and shown to the model

