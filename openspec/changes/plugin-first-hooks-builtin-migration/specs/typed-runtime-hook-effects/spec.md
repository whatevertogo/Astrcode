## ADDED Requirements

### Requirement: hook payloads and effects SHALL be typed per event
Every formal hook event MUST define typed payload and a bounded effect set. JSON MAY be used only at protocol boundaries.

#### Scenario: tool_call payload is dispatched
- **WHEN** runtime dispatches `tool_call`
- **THEN** payload SHALL include session id, turn id, agent id, tool call id, tool name, args, capability spec, working dir, current mode, and step index
- **AND** handlers SHALL return only effects valid for `tool_call`

### Requirement: tool_call effects SHALL distinguish tool denial from turn cancellation
Ordinary permission denial MUST be represented as per-tool denial, not generic turn block.

#### Scenario: tool is blocked by policy
- **WHEN** a `tool_call` hook returns `BlockToolResult`
- **THEN** runtime SHALL skip that tool execution
- **AND** runtime SHALL record a failed tool result with the provided reason

#### Scenario: turn is cancelled
- **WHEN** a hook returns `CancelTurn`
- **THEN** runtime SHALL use the standard turn cancellation path
- **AND** it SHALL NOT convert cancellation into a normal tool result

### Requirement: tool_result effects SHALL apply before persistence
`tool_result` hooks MUST run after tool execution but before the result is appended to durable events or model-visible context.

#### Scenario: result is overridden
- **WHEN** a `tool_result` hook returns `OverrideToolResult`
- **THEN** runtime SHALL persist and expose the overridden result
- **AND** the raw result SHALL NOT be shown to the model unless explicitly preserved in allowed metadata

### Requirement: input effects SHALL be applied by host-session before turn creation
`input` hooks MUST run before turn acceptance and MAY continue, transform input, handle input, or request mode switch.

#### Scenario: input hook switches mode
- **WHEN** input hook returns `SwitchMode(plan)`
- **THEN** host-session SHALL validate and persist the mode change before compiling the turn envelope

