# Agent instructions

## Project context

VijiDeck is a native Rust desktop application using eframe and egui. It provides
system overview, port management, disk cleanup, Docker tools, and environment
inspection. Native packaging is handled by the separate `xtask` crate.

Read `CONTRIBUTING.md` before making changes. Consult `docs/architecture.md` for
module responsibilities and `docs/development.md` for packaging and visual QA.

## Working conventions

Keep changes focused on the requested task and follow the surrounding Rust style.
Keep domain validation independent of UI rendering. Run slow discovery and external
commands through the existing background work and command helpers.

Preserve process identity checks, filesystem boundary validation, and confirmation
flows for destructive operations. Use disposable fixtures when checking deletion,
process termination, environment edits, or Docker actions.

## Validation

For Rust code changes, run the applicable repository checks:

```sh
cargo fmt --check
cargo fmt --manifest-path xtask/Cargo.toml --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --manifest-path xtask/Cargo.toml --locked --all-targets -- -D warnings
cargo test --locked --all-features
```

Include screenshots for UI changes and state which OS was used for verification.
Report any checks that could not be completed and explain the limitation.
