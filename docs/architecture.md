# Kairo Architecture & Technical Design

Kairo provides Git-like execution infrastructure for AI agents. It enables determinism, offline replay, structural diffing, execution checkpointing/branching, evaluation benchmarking, and multi-language agent instrumentation.

---

## 1. Directory & Module Structure

```
src/
├── main.rs                 # Application entry point
├── lib.rs                  # Top-level library and module declarations
│
├── cli/                    # Clap CLI commands and argument handling
│   └── mod.rs
│
├── event/                  # Event model and validation
│   ├── mod.rs
│   └── model.rs
│
├── recorder/               # Recording execution events and run management
│   └── mod.rs
│
├── storage/                # Persistence and file operations
│   ├── mod.rs
│   └── jsonl.rs
│
├── run/                    # Run lifecycle and status tracking
│   └── mod.rs
│
├── replay/                 # Deterministic execution replay
│   └── mod.rs
│
├── compare/                # Structural comparison and diffing of two executions
│   └── mod.rs
│
├── checkpoint/             # Checkpoints, execution branching, and lineage
│   └── mod.rs
│
├── eval/                   # Evaluation engine, metrics, and regression testing
│   ├── mod.rs
│   ├── case.rs
│   ├── metrics.rs
│   └── regression.rs
│
└── server/                 # Axum & Tokio asynchronous HTTP API server
    └── mod.rs
```

---

## 2. Core Subsystems

### Event Model (`src/event/`)
- Every observable action within an agent's execution is captured as an immutable `Event`.
- Events contain an auto-generated UUIDv4 (`event_id`), run reference (`run_id`), UTC ISO-8601 timestamp, `event_type` (`run.started`, `llm.request`, `llm.response`, `tool.started`, `tool.completed`, `run.completed`, etc.), and structured JSON `payload`.

### Storage Engine (`src/storage/`)
- Pure append-only JSON Lines (`.jsonl`) persistence.
- Guarantees that historical events are never overwritten by append operations.
- Tolerant parsing handles malformed lines safely without crashing.

### Run & Recorder (`src/run/` & `src/recorder/`)
- Enforces strict lifecycle state transitions: `Running -> Completed` or `Running -> Failed`.
- Rejects invalid transitions (e.g., duplicate completions or recording to a finished run).

### Replay Engine (`src/replay/`)
- Loads previous executions and steps through them deterministically without executing live shell commands or contacting LLM APIs.
- Detects divergence when an agent deviates in its event sequence or tool input/output.

### Execution Comparison (`src/compare/`)
- Computes structural differences between two execution runs.
- Ignores non-deterministic volatile metadata (timestamps, UUIDs) and highlights semantic modifications, additions, and deletions.

### Checkpoints & Branching (`src/checkpoint/`)
- Pins execution state to a specific event step.
- Branches new runs from existing checkpoints, preserving immutable history and tracking ancestral lineage.

### HTTP Core API & Python SDK (`src/server/` & `sdk/python/`)
- Built with Axum and Tokio for high-throughput asynchronous request handling.
- Python SDK provides zero-dependency client integration (`KairoClient`, `Run`, context managers).

### Evaluation Engine (`src/eval/`)
- Derives repeatable benchmark test cases (`EvalCase`) directly from recorded runs.
- Measures duration and captures token usage only when supplied by the provider.
- Compares baseline vs. candidate runs to detect regressions (`RegressionReport`).
