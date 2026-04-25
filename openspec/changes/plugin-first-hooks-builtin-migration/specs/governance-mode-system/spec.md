## ADDED Requirements

### Requirement: plan mode entry SHALL occur before turn envelope compilation
Automatic or command-driven plan mode entry that should affect the next turn MUST be applied by host-session before turn-scoped governance envelope is compiled.

#### Scenario: input hook switches to plan mode
- **WHEN** a user input triggers planning hook and requests `plan`
- **THEN** host-session SHALL validate transition and append durable mode event before runtime turn creation
- **AND** the compiled envelope SHALL use plan mode

#### Scenario: runtime turn_start attempts mode switch
- **WHEN** runtime `turn_start` hook returns mode switch
- **THEN** runtime SHALL reject that effect as invalid for `turn_start`
- **AND** current turn envelope SHALL remain unchanged

