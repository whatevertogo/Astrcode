## ADDED Requirements

### Requirement: plugin-host SHALL bind hook descriptors to executable handlers
`plugin-host` MUST turn every enabled hook contribution into an executable registration before it enters the active snapshot. A hook contribution MUST include event, stage, dispatch mode, failure policy, priority, entry ref, and declared payload/effect contract.

#### Scenario: builtin hook binds to in-process handler
- **WHEN** a builtin plugin declares `entry_ref=builtin://hooks/<id>`
- **THEN** plugin-host SHALL resolve it through the builtin hook executor registry
- **AND** the hook SHALL be callable without an external process hop

#### Scenario: external hook binding fails
- **WHEN** an external plugin declares a hook entry that its backend cannot handle
- **THEN** candidate snapshot staging SHALL fail
- **AND** the previous active snapshot SHALL remain active

### Requirement: hook dispatch SHALL use snapshot-consistent registrations
Hook dispatch MUST use the snapshot selected for the session/turn. Reload SHALL NOT change handlers already bound to an in-flight turn.

#### Scenario: reload during turn
- **WHEN** a turn starts with snapshot `A` and reload commits snapshot `B`
- **THEN** all hook dispatches inside that turn SHALL continue using snapshot `A`
- **AND** new turns SHALL use snapshot `B`

### Requirement: external plugins SHALL expose hook handlers through protocol messages
The plugin protocol MUST define hook dispatch request and response messages so an external plugin can receive hook events and return typed effects.

#### Scenario: external hook receives dispatch
- **WHEN** plugin-host dispatches a hook bound to an external backend
- **THEN** it SHALL send correlation id, hook id, event key, snapshot id, and payload
- **AND** the response SHALL contain only effects allowed by that event

### Requirement: builtin hook registration SHALL support function-style handlers
Builtin hook authors MUST be able to register event handlers through typed function registration helpers. Implementing a dedicated struct per hook MAY be supported internally, but SHALL NOT be required for simple builtin hooks.

#### Scenario: builtin plugin registers tool_call handler with a closure
- **WHEN** a builtin plugin calls a helper such as `on_tool_call("builtin-plan-mode.block-writes", handler)`
- **THEN** plugin-host SHALL bind that handler into the active hook registry
- **AND** the handler SHALL receive a typed `tool_call` context rather than raw JSON

#### Scenario: registry stores handlers behind internal executor abstraction
- **WHEN** plugin-host stages the active snapshot
- **THEN** function-style handlers MAY be erased into a common executor representation
- **AND** this internal representation SHALL NOT leak as mandatory plugin-author boilerplate

### Requirement: HookContext SHALL be a restricted invocation context
Builtin hook handlers MAY receive a `HookContext`, but it MUST be limited to typed event metadata, read-only host views, cancellation state, and validated action requests. It SHALL NOT expose mutable session state, event stores, plugin snapshot mutation, or unrestricted host services.

#### Scenario: handler needs session state
- **WHEN** a hook handler needs current mode or cancellation state
- **THEN** it SHALL read that data from typed payload or `HookContext` read-only views
- **AND** it SHALL return a typed effect for any behavior change

#### Scenario: handler attempts direct durable mutation
- **WHEN** a hook handler attempts to append a session event or mutate session state directly through `HookContext`
- **THEN** the API SHALL not expose such a method
- **AND** owner code SHALL remain the only path that applies durable mutations
