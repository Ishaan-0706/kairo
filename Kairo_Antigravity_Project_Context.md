# Kairo --- Project Context for Antigravity

> **Purpose of this document:** Give Antigravity a durable understanding
> of what Kairo is, why it exists, how it should be built, and how to
> work with its developer.
>
> Treat this file as the project's guiding context. Read it before
> making architectural or implementation changes. The developer is
> learning Rust and systems programming, so teaching and verification
> are part of the work.

------------------------------------------------------------------------

## 1. Project identity

**Name:** Kairo\
**Category:** Open-source AI-agent infrastructure / developer tooling\
**Core implementation:** Rust\
**Initial integration SDK:** Python\
**Initial architecture:** Local-first modular monolith\
**Long-term direction:** Git-like execution history, replay, branching
and comparison for AI agents

### One-sentence description

Kairo records, inspects, replays and compares observable AI-agent
executions so developers can understand what happened, reproduce
failures and investigate differences between runs.

### Core promise

**Every agent execution should be inspectable, comparable and
recoverable.**

Kairo is not intended to be another generic tracing dashboard. Its
distinctive direction is Git-like execution versioning: recorded
histories, checkpoints, branches, replay and meaningful diffs.

------------------------------------------------------------------------

## 2. The problem Kairo addresses

An AI agent may perform a sequence of operations:

1.  Receive a task.
2.  Send a request to an LLM.
3.  Receive an LLM response.
4.  Call one or more tools.
5.  Read or modify files.
6.  Encounter errors and retry.
7.  Produce a final result.

When the agent behaves incorrectly, developers need to answer questions
such as:

-   What happened during this execution?
-   Which operation failed?
-   What inputs and outputs were observed?
-   Can I reproduce the same interaction sequence without calling
    external services again?
-   What changed between two executions?
-   Where did two executions first meaningfully diverge?
-   Can I create a new execution path from an earlier checkpoint?

Kairo should provide the underlying data and operations to answer these
questions.

### Important scope boundary

Kairo records **observable execution data**: requests, responses, tool
inputs and outputs, timestamps, statuses, errors, metadata and
explicitly captured application events.

Kairo must not attempt to capture private model reasoning or hidden
chain-of-thought. It should not claim that replay reconstructs a model's
internal reasoning or guarantees identical model computation.

------------------------------------------------------------------------

## 3. Product goals

### Primary goals

1.  Record agent execution events in a consistent, framework-independent
    format.
2.  Persist event histories reliably.
3.  Inspect executions without manually reading raw log files.
4.  Replay recorded interactions using saved responses rather than
    contacting external services.
5.  Compare two executions and identify meaningful differences.
6.  Eventually support checkpoints and branches, similar to
    version-control workflows.
7.  Provide a Python SDK so Python-based agents can integrate with the
    Rust core.

### Secondary goals

-   Turn recorded failures into repeatable evaluation cases.
-   Measure execution duration and provider-reported token usage.
-   Support safe data redaction and configurable capture.
-   Keep event data portable and documented.
-   Build a foundation that can later support larger workloads and
    alternative storage backends.

### Non-goals for the initial MVP

Do not build these at the beginning:

-   A web dashboard.
-   Kubernetes deployment.
-   Distributed databases or multi-node synchronization.
-   GPU management.
-   A new LLM or agent framework.
-   Automatic execution of recorded shell commands during replay.
-   Complex AI-generated explanations of traces.

Do not add infrastructure merely because it may be useful in a future
version.

------------------------------------------------------------------------

## 4. Intended user workflow

The intended developer workflow will eventually look conceptually like
this:

``` text
1. Instrument an agent with Kairo.
2. Run the agent.
3. Kairo records events and stores the run.
4. Developer lists and inspects the run.
5. Developer replays recorded interactions.
6. Developer compares two runs.
7. Developer creates a checkpoint or branch when supported.
8. Developer converts useful failures into repeatable evaluations.
```

The CLI shown below is an intended interface, not a promise that all
commands already exist:

``` bash
kairo run start
kairo runs
kairo inspect <run-id>
kairo replay <run-id>
kairo diff <run-id-a> <run-id-b>
```

------------------------------------------------------------------------

## 5. Core domain concepts

Antigravity must use these terms consistently and explain them when they
are introduced.

  -----------------------------------------------------------------------
  Term                                Meaning in Kairo
  ----------------------------------- -----------------------------------
  Agent                               The application or program
                                      performing an AI-assisted task.

  Run / execution                     One instance of an agent performing
                                      a task.

  Event                               A recorded occurrence during a run.

  Event ID                            Unique identifier for one event.

  Run ID                              Identifier shared by events
                                      belonging to one run.

  Payload                             Event-specific data, such as a tool
                                      input or response.

  Event log                           Ordered persisted collection of
                                      events.

  Recorder                            Component that accepts and records
                                      events.

  Storage backend                     Component responsible for
                                      persisting and loading data.

  Replay                              Reusing recorded interactions to
                                      reproduce an observable sequence.

  Divergence                          A meaningful difference between
                                      expected and actual or between two
                                      runs.

  Checkpoint                          A reference to a defined point in
                                      an execution history.

  Branch                              A new execution path based on an
                                      earlier checkpoint.

  Evaluation                          A repeatable test of an agent
                                      against a defined scenario.
  -----------------------------------------------------------------------

------------------------------------------------------------------------

## 6. Initial architecture

Start with a **modular monolith**: one application with clearly
separated modules. Do not start with microservices.

Conceptual architecture:

``` text
             AGENT / TEST PROGRAM
                      |
                      v
                KAIRO INTERFACE
             (initially Rust API/CLI)
                      |
                      v
                KAIRO CORE
          +-----------+-----------+
          |           |           |
       Run Manager  Recorder   Event Model
          |           |           |
          +-----------+-----------+
                      |
                      v
                 STORAGE LAYER
                (initially JSONL)
                      |
          +-----------+-----------+
          |           |           |
        Inspect     Replay      Compare
                      |
                Later: Branching,
                 Checkpoints and
                   Evaluation
```

### Responsibilities

**Event model** - Defines stable event structures and event types. -
Handles serialization and deserialization. - Validates required fields
and supported schema versions.

**Run manager** - Creates runs and assigns IDs. - Tracks run lifecycle
and status. - Associates events with the correct run. - Rejects invalid
lifecycle transitions.

**Recorder** - Accepts events from callers. - Adds or validates common
metadata. - Passes events to storage. - Exposes a simple API without
leaking storage details.

**Storage** - Initially writes JSON Lines (JSONL). - Appends new events
rather than rewriting the full history. - Loads event histories. -
Handles malformed or incomplete records deliberately. - Later may use
SQLite for metadata and indexing.

**CLI** - Provides developer-facing commands. - Calls core
functionality; it must not duplicate domain or storage logic.

**Replay** - Reads a stored run in order. - Supplies recorded LLM/tool
responses to a controlled replay consumer. - Does not contact external
providers by default. - Does not execute recorded shell commands or tool
side effects. - Reports missing or mismatched events.

**Compare** - Compares two run histories using a documented matching
strategy. - Ignores volatile fields such as generated IDs and timestamps
by default when determining semantic equality. - Reports added, removed
and modified events and the first meaningful divergence.

**Python SDK (later)** - Provides a friendly Python API. - Communicates
with the Rust core through a documented local HTTP interface
initially. - Does not reimplement Rust core logic.

------------------------------------------------------------------------

## 7. Event data model

The event envelope should be stable and versioned. Event-specific
details belong in the payload.

Conceptual example:

``` json
{
  "schema_version": 1,
  "event_id": "event-001",
  "run_id": "run-001",
  "timestamp": "2026-11-14T10:00:00Z",
  "event_type": "tool.completed",
  "parent_event_id": "event-000",
  "payload": {
    "tool": "read_file",
    "status": "success",
    "output": "Hello world"
  }
}
```

Potential initial event types:

``` text
run.started
run.completed
run.failed
llm.request
llm.response
tool.started
tool.completed
tool.failed
file.changed
checkpoint.created
```

These are initial candidates. Keep event-type design extensible and
document any changes.

### Data-model guidance

-   Use unique IDs for events and runs.
-   Store timestamps in a clearly defined format, preferably UTC.
-   Preserve event order within a run.
-   Distinguish event identity from event content.
-   Represent event-specific data in a payload.
-   Add schema versioning before persisted data becomes widely used.
-   Define behavior for unknown event types and unsupported schema
    versions.
-   Avoid storing secrets and unrestricted environment variables.
-   Support configurable redaction before production use.

------------------------------------------------------------------------

## 8. Functional requirements

These are product capabilities. Implement them in phases rather than all
at once.

### Run management

-   Create a run with a unique ID.
-   Store run name, agent identity and metadata.
-   Track running, completed, failed and (later) cancelled status.
-   Reject invalid lifecycle transitions.
-   List runs and retrieve run metadata.

### Event recording

-   Record events associated with a run.
-   Capture LLM request/response metadata and configured payloads.
-   Capture tool invocation inputs, outputs and statuses.
-   Capture errors and retries.
-   Record timestamps and causal/parent references where available.
-   Support custom application events.
-   Validate event structure.

### Inspection

-   List runs.
-   Show a run's event history.
-   Filter events by type, status or time.
-   Inspect event payloads and metadata.
-   Show failures.
-   Export run histories.

### Replay

-   Load a stored run.
-   Process events in order.
-   Reuse saved LLM/tool responses.
-   Avoid external API calls and side effects by default.
-   Detect missing, unexpected or mismatched events.
-   Produce a replay report.

### Comparison

-   Compare two runs.
-   Identify added, removed and modified events.
-   Identify the first meaningful difference.
-   Explain the field-level differences.

### Later capabilities

-   Checkpoints and branches.
-   Execution lineage.
-   Python SDK and HTTP API.
-   Evaluation datasets and repeatable tests.
-   Latency and token-usage measurements.
-   Regression detection and model/prompt comparisons.

------------------------------------------------------------------------

## 9. Non-functional requirements

Treat these as engineering quality goals. Do not claim a requirement is
satisfied without a test or measurement.

### Performance

-   Recording should add low overhead to the monitored agent.
-   Storage should avoid unnecessary blocking.
-   Large histories should be processed without loading everything into
    memory when practical.
-   Replay and inspection should remain responsive as histories grow.

### Reliability and integrity

-   Successfully committed events should survive normal process
    restarts.
-   Interrupted writes and malformed records must be handled
    deliberately.
-   Corruption should be detectable.
-   Event ordering and causal relationships must be preserved.
-   Recording errors should be surfaced clearly; do not silently lose
    data.
-   Define whether the recorder is synchronous, buffered or
    asynchronous, and document its durability trade-offs.

### Scalability

-   Support multiple concurrent runs within the limits of the initial
    local architecture.
-   Avoid assumptions that prevent future storage backends.
-   Do not prematurely introduce distributed systems.

### Security and privacy

-   Redact secrets and sensitive values.
-   Make captured fields configurable.
-   Define data retention and deletion behavior.
-   Protect local API access.
-   Add authentication and authorization before exposing a network
    service beyond a trusted local environment.
-   Never log secrets in error messages.

### Maintainability and portability

-   Keep modules independently testable.
-   Keep event format documented and versioned.
-   Avoid coupling the core to one agent framework.
-   Keep storage behind an interface where justified.
-   Provide portable exports and reproducible examples.

### Developer experience

-   Provide clear CLI errors.
-   Document APIs and schemas.
-   Keep integration changes small.
-   Explain setup and troubleshooting.
-   Provide useful examples.

------------------------------------------------------------------------

## 10. Technology choices

Use dependencies only when a phase needs them. Verify current versions
and compatibility before adding packages.

  Technology   Intended use
  ------------ ---------------------------------------------------
  Rust         Core implementation
  Cargo        Build and package management
  Serde        Rust serialization framework
  serde_json   JSON serialization
  UUID         Unique identifiers
  Tokio        Async runtime when asynchronous I/O/API is needed
  Clap         CLI argument parsing
  tracing      Internal diagnostics
  JSONL        Initial append-only event storage
  SQLite       Later metadata/indexing if justified
  Python       Initial agent integration SDK
  HTTP         Initial local SDK-to-core communication

Do not add Tokio, SQLite, HTTP frameworks or other dependencies just to
prepare for a hypothetical future. Explain why each dependency is needed
at the point it is introduced.

------------------------------------------------------------------------

## 11. Phased development roadmap

**Strict rule:** Work on one small phase at a time. Do not implement the
next phase until the developer understands and manually verifies the
current phase.

### Phase 0 --- Environment and Rust project foundation

Build: - Check Rust and Cargo installation. - Initialize the
repository. - Create the smallest working Rust executable. - Set up Git
and a basic README.

Learn: - Rust toolchain. - Cargo and `Cargo.toml`. - Crates and project
structure. - Compilation and execution. - Basic Git workflow.

Manual verification:

``` bash
rustc --version
cargo --version
cargo check
cargo build
cargo run
```

Acceptance criteria: - The program builds and runs. - The developer can
explain Cargo's role and the project files.

### Phase 1 --- Event model

Build: - Event struct. - Event type representation. - IDs and
timestamps. - JSON serialization and deserialization. - Basic
validation.

Learn: - Structs, enums, types and ownership as needed. - Serde. -
Serialization versus deserialization. - UUIDs and timestamps.

Manual verification: - Run the example. - Inspect generated JSON. -
Verify deserialization. - Try invalid or incomplete data.

Acceptance criteria: - A valid event round-trips Rust → JSON → Rust. -
Invalid data produces a useful error.

### Phase 2 --- JSONL storage

Build: - Append events to a JSONL file. - Read events back. - Handle
empty, missing and malformed files. - Keep storage code separate from
event definitions.

Learn: - File I/O. - Append mode. - Buffering. - Persistence. -
`Result`, errors and filesystem behavior.

Manual verification: - Write multiple events. - Inspect the file with
`cat`. - Stop and restart the program. - Confirm prior events remain
readable. - Test malformed lines.

Acceptance criteria: - Multiple events persist across process
restarts. - Existing events are not overwritten by append operations. -
Failure behavior is documented and tested.

### Phase 3 --- Run manager and recorder

Build: - Create and finish runs. - Track run status. - Record events
against the correct run. - Enforce valid lifecycle transitions.

Learn: - Modules and abstractions. - State and state transitions. - API
boundaries. - Error handling.

Manual verification: - Create a run. - Record several events. - Complete
or fail the run. - Inspect JSONL and verify run IDs. - Try invalid
transitions.

Acceptance criteria: - A complete run can be recorded and loaded. -
Invalid run lifecycle operations are rejected.

### Phase 4 --- CLI

Build: - `runs` - `inspect <run-id>` - `run start`

Use Clap when needed.

Learn: - CLI argument parsing. - Subcommands. - Separating user
interface from business logic.

Manual verification: - Run each command. - Try invalid commands and
unknown IDs. - Try an empty data directory.

Acceptance criteria: - A developer can create, list and inspect runs
from the terminal. - CLI code does not duplicate core logic.

### Phase 5 --- Replay

Build: - Load an execution. - Consume its events in sequence. - Reuse
recorded responses. - Detect missing or mismatched interactions. -
Produce a replay report.

Learn: - Determinism and its limits. - Mocking. - State machines. -
Sequence validation. - Divergence.

Manual verification: - Record a mock-agent run. - Replay it with no
external provider access. - Modify a fixture and replay again. - Confirm
divergence is reported. - Confirm no recorded shell command or side
effect executes.

Acceptance criteria: - A mock execution can replay from saved
responses. - Replay is safe by default and detects mismatches.

### Phase 6 --- Run comparison

Build: - Compare two run histories. - Ignore volatile identity/time
fields by default. - Report added, removed and modified events. -
Identify first meaningful divergence.

Learn: - Sequence comparison. - Structural diffs. - Event matching
strategies.

Manual verification: - Create two identical runs. - Create two runs with
one changed payload. - Test extra, missing and reordered events.

Acceptance criteria: - The comparison output correctly identifies
meaningful differences.

### Phase 7 --- Checkpoints and branches

Build: - Checkpoint references. - Branch creation from a checkpoint. -
Parent-child lineage. - History reconstruction.

Learn: - Checkpointing. - Branching. - Immutable history. - Shared
history and copy-on-write trade-offs.

Manual verification: - Create a run and checkpoint. - Branch from the
checkpoint. - Add branch-specific events. - Verify the original history
remains unchanged.

Acceptance criteria: - A branch can refer to earlier history without
corrupting its parent.

### Phase 8 --- Python SDK and local API

Build: - Documented local HTTP API. - Python client to create runs and
submit events. - Simple mock-agent example. - Clear network and
validation errors.

Learn: - HTTP, client/server architecture, APIs and SDKs. -
Request/response handling. - Network failure and retry behavior.

Manual verification: - Start the Rust service. - Run the Python
example. - Inspect its run through the CLI. - Test unavailable server
and invalid requests.

Acceptance criteria: - A Python program can create and record a run
through the documented interface.

### Phase 9 --- Evaluation engine

Build: - Reusable test cases from recorded runs. - Expected observable
outcomes. - Execution duration and provider-reported token usage. -
Regression comparison and reports.

Learn: - Evaluation datasets. - Metrics and baselines. - Benchmarking. -
Regression testing.

Manual verification: - Run a deterministic mock agent against a test
case. - Deliberately change its output. - Confirm the regression is
detected.

Acceptance criteria: - Kairo can evaluate repeatable mock scenarios and
identify a deliberately introduced regression.

------------------------------------------------------------------------

## 12. Testing strategy

Every phase must include three kinds of testing.

### A. Unit tests

Test one function or component in isolation.

Examples: - Event serialization. - Event validation. - Run state
transitions. - Event comparison logic.

Typical command:

``` bash
cargo test
```

### B. Integration tests

Test multiple components together.

Examples: - Recorder writes events through the storage layer. - CLI
lists stored runs. - Replay loads a run and consumes saved responses. -
HTTP API accepts events from the SDK.

Use temporary directories for filesystem tests. Tests should not depend
on the developer's real Kairo data.

### C. Manual end-to-end tests

Provide exact terminal commands, expected output and failure cases. The
developer must perform these manually and understand the result.

### Failure testing

Include appropriate cases for each component: - Missing data
directory. - Empty event file. - Malformed JSONL line. - Interrupted or
failed write. - Duplicate IDs. - Unknown run ID. - Invalid lifecycle
transition. - Missing replay response. - Mismatched event sequence. -
Server unavailable. - Large event history.

Do not mark manual checks as passed merely because automated tests
passed.

### Testing report format

After each implementation phase, report:

1.  What was implemented.
2.  Files created or modified.
3.  Important functions and their responsibilities.
4.  Concepts introduced.
5.  Automated tests actually run and their exact outcomes.
6.  Manual commands for the developer.
7.  Expected output.
8.  Failure cases to try.
9.  Known limitations and remaining risks.
10. Acceptance criteria still not met.

Never fabricate test output or claim tests were run when they were not.

------------------------------------------------------------------------

## 13. How Antigravity must work with the developer

The developer is learning Rust, systems programming and distributed/AI
infrastructure. The developer wants to understand the engineering, not
just receive generated code.

### Mandatory working rules

1.  Read this document before starting work.
2.  Inspect the existing repository before editing.
3.  Work only on the phase explicitly requested.
4.  Break a phase into small implementation steps if necessary.
5.  Explain the design before making significant changes.
6.  Explain new jargon in simple language when it first appears.
7.  Explain why each component exists and how data flows through it.
8.  Explain important functions: responsibility, inputs, outputs,
    dependencies and failure modes.
9.  Explain Rust concepts introduced by the code, including ownership,
    borrowing, lifetimes, traits, generics, async or error handling when
    relevant.
10. Prefer simple, idiomatic code over unnecessary abstraction.
11. Do not introduce dependencies without explaining their purpose and
    trade-offs.
12. Do not silently change architecture or event formats.
13. Preserve working behavior and existing tests.
14. Run formatting, compilation and relevant automated tests after
    changes.
15. Provide manual test instructions with expected results.
16. Do not proceed to the next phase without explicit permission from
    the developer.

### Required response structure after each task

Use this structure:

#### 1. Goal

What this small step accomplishes.

#### 2. Concept lesson

Explain the new terms and relevant fundamentals.

#### 3. Design

Explain the components and data flow before or alongside implementation.

#### 4. Implementation

List changed files and explain important code.

#### 5. Automated verification

Show commands actually run and their results. Separate failures from
successes.

#### 6. Manual testing

Give exact commands, expected output and negative/failure tests.

#### 7. What to understand

Give a short list of questions the developer should be able to answer.

#### 8. Completion status

State which acceptance criteria are met and which are not.

Then stop and wait.

### Teaching style

-   Start from fundamentals.
-   Use small examples.
-   Explain why before diving into implementation details.
-   Avoid unexplained jargon.
-   Keep explanations precise and not unnecessarily long.
-   Do not dump the entire codebase at once.
-   If the developer asks a question, answer it before continuing
    implementation.

------------------------------------------------------------------------

## 14. Engineering quality principles

-   Prefer correctness and clear data semantics over premature
    performance optimization.
-   Measure recording overhead before claiming it is low.
-   Define durability guarantees instead of casually claiming data is
    safe.
-   Distinguish replay of recorded interactions from reproducing model
    internals.
-   Make replay safe by default: no real network calls or side effects
    unless a future explicit, guarded mode is designed.
-   Treat event schema evolution as a first-class concern.
-   Never silently discard events or errors.
-   Avoid logging secrets and private data.
-   Keep the core independent of specific LLM providers and agent
    frameworks.
-   Build a modular monolith first; introduce distributed architecture
    only when requirements justify it.

------------------------------------------------------------------------

## 15. First task for Antigravity

When this document is added to the repository, do not immediately
implement Kairo.

First:

1.  Read this document fully.
2.  Inspect the repository and available Rust toolchain.
3.  Summarize Kairo's purpose, architecture, scope and development rules
    in your own words.
4.  Identify the current project state without assuming files or
    features exist.
5.  Propose the smallest next action for **Phase 0**.
6.  Wait for the developer's approval before making changes.

**The immediate objective is not to build all of Kairo. It is to build
one reliable piece, understand it, test it and then continue.**
