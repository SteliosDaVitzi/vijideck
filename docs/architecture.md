# Architecture and maintenance audit

Reviewed 2026-11-07. Scope: the Rust desktop application, background work,
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
| `ports.rs` / `process_control.rs` | Socket inventory and ownership validation / shared process identity checks and native termination |
| `cleanup.rs` / `cleanup_ui.rs` | Filesystem discovery and validated deletion / UI state and confirmation |
| `docker/model.rs` | Container validation, action requests, search, and deterministic Compose grouping |
| `docker/client.rs` | Docker CLI invocation, inventory loading, and action execution |
| `docker.rs`, `docker_resources.rs` | Container and resource views and background result handling |
| `docker_logs.rs`, `docker_terminal.rs` | Streaming logs and interactive terminal lifecycles |
| `command.rs` | Bounded capture and deadlines for finite external commands |
| `overview.rs`, `gpu.rs`, `volumes.rs` | Telemetry, hardware inventory, and shared volume policy |
| `processes.rs` | Process inventory, grouping, sorting, and process view |
| `environment*.rs`, `profile.rs` | Tool catalog, discovery, provenance and views / persistent settings adapters and serialized profile replacement |
| `worker.rs` | Finite-job completion, panic reporting and disconnected-channel handling |
| `logs.rs` / `logs_ui.rs`, `logs_list.rs`, `logs_access.rs` | OS log providers / presentation and explicit access requests |
| `services.rs`, `services/client/` / `services_ui.rs` | Service identities, capabilities and stale-state validation; bounded platform commands / service inventory, inspector and confirmations |
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

## SOLID refactoring follow-up

Environment variable reads and PATH changes run in background jobs. Persistent
settings live outside the views; Unix variable and PATH edits share a serialized
profile updater. It writes a temporary sibling, syncs it, checks for changed
contents/identity, and atomically replaces the resolved target, retaining symlinks
and Unix ownership, mode and extended attributes (including ACLs). Conflict detection
is best effort: an unrelated writer can still race the final check and rename.
Metadata-copy failures leave the original file intact. Windows authenticated writes
retain their existing completion semantics; read-only PowerShell inventory uses bounded capture.

Finite jobs report panic/spawn failures through `worker.rs`. Cleanup keeps its
streaming progress events and adds a separate job-completion channel, so a retained
sender cannot hide worker failure. Failed mutations are never retried automatically.
Long-lived port/process workers clear busy state and report disconnection; restarting
the application is required to recreate a failed long-lived worker.

Service adapters identify their manager; shared capability and state classification
powers both action validation and presentation. Native state strings remain available
for display and stale-snapshot comparisons. Systemd's manually startable disabled
units remain distinct from Windows/launchd disabled services. Both process views now
use `process_control.rs`; socket ownership remains an additional port-specific check.
Developer-tool discovery and presentation use one metadata catalog with stable cache
names, categories, grouping, primary-version roles and release links.

Follow-up validation on Linux: 89 tests passed, three intentionally ignored;
strict app and xtask Clippy passed. Environment and compact service previews were
captured using disposable/synthetic data. Changed Rust files pass rustfmt; the
repository-wide app and xtask formatting checks still report pre-existing
formatting differences in untouched source. Native Windows/macOS testing remains
outstanding.

## Remaining maintenance considerations

- `cleanup_ui.rs`, `environment.rs`, and `processes.rs` still combine view state
  with substantial rendering or discovery logic. Extract cohesive responsibilities
  as those features change; the Docker model/view split provides a pattern.
- Continuous telemetry, logs and interactive terminal streams retain their own
  cancellation and process lifecycles. The finite-job helper does not replace them.
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
