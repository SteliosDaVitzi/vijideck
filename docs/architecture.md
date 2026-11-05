# Architecture and maintenance audit

Reviewed 2026-11-05. Scope: the Rust desktop application, background work,
platform integrations, persistence, tests, and native packaging workflow.

## Design

VijiDeck is a single native Rust executable. Keeping one crate is appropriate
for its size and deployment model; these internal boundaries do not need separate
packages or an async runtime.

| Area | Responsibility |
| --- | --- |
| `main.rs` | Native window setup and application startup |
| `app.rs`, `app/tool.rs` | Application composition, typed navigation, preferences, and event dispatch |
| `app/ports_view.rs` | Port table, inspector, notices, and stop confirmation rendering |
| `ports.rs` | Socket inventory, process identity checks, and termination |
| `cleanup.rs` / `cleanup_ui.rs` | Filesystem discovery and validated deletion / UI state and confirmation |
| `docker/model.rs` | Container validation, action requests, search, and deterministic Compose grouping |
| `docker/client.rs` | Docker CLI invocation, inventory loading, and action execution |
| `docker.rs`, `docker_resources.rs` | Container and resource views and background result handling |
| `docker_logs.rs`, `docker_terminal.rs` | Streaming logs and interactive terminal lifecycles |
| `command.rs` | Bounded capture and deadlines for finite external commands |
| `overview.rs`, `gpu.rs`, `volumes.rs` | Telemetry, hardware inventory, and shared volume policy |
| `processes.rs` | Process inventory, grouping, sorting, and process view |
| `environment*.rs` | Tool discovery, installation provenance, icons, PATH, and environment editing |
| `logs.rs` / `logs_ui.rs`, `logs_list.rs`, `logs_access.rs` | OS log providers / presentation and explicit access requests |
| `storage.rs` | SQLite schema, preferences, and disposable discovery caches |
| `ui.rs` | Shared presentation primitives |
| `xtask` | Native packaging, separate from runtime code |

Dependencies should flow from views to domain logic and platform adapters.
Domain validation and grouping must remain callable without constructing an egui
context. External command capture must not depend on UI or feature-specific error
messages. Workers send owned results back to UI state; workers do not mutate the
view directly.

## Findings and changes

1. **High: inconsistent subprocess lifecycle handling.** Tool version detection
   waited for exit before draining pipes, allowing a sufficiently verbose tool to
   fill a pipe and stall until timeout. Docker retained unlimited output, and its
   reader joins could wait past its process deadline if descendants retained the
   pipes. Log readers discarded pipe errors and reader panics as empty output.
   `command::capture` now drains both streams concurrently, limits each stream,
   propagates failures, and applies one deadline through both exit and pipe
   completion. An owned guard kills and reaps the direct child on failure.
   Docker, version detection, log queries, and Windows hardware inventory use it.
   Callers retain their own timeouts, byte budgets, exit-status interpretation,
   and user-facing errors. Log queries preserve permission classification.
2. **Medium: Docker domain and transport coupled to its view.** Parsing, grouping,
   action requests, CLI argument construction, and process execution lived in the
   UI module; the resource view called into that UI module for transport. Model
   and client modules now own those responsibilities. Pure regression tests moved
   with the model. Search query normalization occurs once per row-list build.
   Group actions still retain members hidden by search filters.
3. **Medium: navigation encoded as integers.** Seven tool IDs were repeated across
   routing, labels, shortcuts, previews, and status text. `Tool` replaces these
   integers. An exhaustive render match makes missing routing a compiler error,
   and titles live with the enum.
4. **Medium: application shell mixed with port-view implementation.** Port UI and
   its presentation helpers now live in `app/ports_view.rs`. The app retains
   composition, navigation, lifecycle, and worker event handling. This is a
   rendering extraction; port state is still owned by the app.
5. **Verification gap: preview configuration excluded from CI checks.** CI now
   checks and tests all features so screenshot-only branches compile and run
   alongside the normal build.

The existing cleanup boundary is valuable: selection/confirmation belongs to the
view, while fresh filesystem validation and deletion belong to the backend.
Likewise, caches are revalidated rather than trusted as deletion authority.
These mechanisms and the process identity protections were preserved.

## Remaining maintenance considerations

- `cleanup_ui.rs`, `environment.rs`, and `processes.rs` still combine view state
  with substantial rendering or discovery logic. Extract cohesive responsibilities
  as those features change; the Docker model/view split provides a pattern.
- Background workers use several channel/state patterns. Some views only handle
  successful channel receipt and can retain a busy state if a worker panics.
  A future worker-lifecycle abstraction should explicitly model disconnection and
  cancellation, while preserving each feature's different streaming semantics.
- The finite-command helper supervises the direct child, not an entire process
  tree. Inherited-pipe reader threads can remain alive until descendants close
  their handles, although the calling worker returns at its deadline. Stronger
  process-tree guarantees require platform-specific process groups/job objects.
- Interactive terminals and continuous log streams intentionally have separate
  lifecycles. Windows environment/PATH editing still uses its existing PowerShell
  adapter; changing timeouts for writes needs explicit handling of uncertain
  completion after an authentication or execution timeout.
- Native Windows/macOS behavior and compositor/tray integration require native
  verification. Linux compilation cannot establish those runtime guarantees.

## Validation

New tests cover exact output limits, overflow, pipe read errors, concurrent large
stdout/stderr, nonzero exit codes, child timeout, and inherited-pipe timeout.
Existing tests cover Compose grouping/action membership, UI layouts, cleanup
validation, platform volume rules, preferences, and process identity protection.

Use the repository checks from a toolchain containing rustfmt and Clippy:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo test --manifest-path xtask/Cargo.toml --locked
```

In the audit environment, two existing socket integration tests failed before
changes: binding a loopback listener returned `Operation not permitted`, and the
child-listener fixture consequently could not start. Keep these tests enabled in
CI; skip them only for local runs under equivalent sandbox restrictions.

Local results after refactoring: `cargo check --locked --all-targets
--all-features` passed; the all-feature test suite passed 64 tests with the two
sandbox-blocked integration tests explicitly skipped and three pre-existing
ignored tests; the packaging tool test target compiled and passed (zero tests).
`git diff --check` passed. Rustfmt and Clippy are not installed in this environment;
attempting to download them failed because the package host could not be resolved.
Formatting/lint completion and native Windows/macOS validation remain outstanding.
