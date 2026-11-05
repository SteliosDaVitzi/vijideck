# Contributing to VijiDeck

Start with an issue for substantial changes. For bugs, include your OS and
architecture, VijiDeck version, reproduction steps, expected behavior, and actual
behavior. Remove secrets, personal paths, environment values, and private log or
container data from reports and screenshots. Follow [SECURITY.md](SECURITY.md)
for vulnerabilities.

See the [README](README.md) for setup and [development guide](docs/development.md)
for packaging and visual previews. Before submitting a pull request, run:

```sh
cargo fmt --check
cargo fmt --manifest-path xtask/Cargo.toml --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --manifest-path xtask/Cargo.toml --locked --all-targets -- -D warnings
cargo test --locked --all-features
```

Keep changes focused. Explain behavior changes and how you verified them. Include
screenshots for UI changes and the OS used for testing. Use disposable fixtures
for deletion, process termination, environment edits, and Docker actions. Add
regression coverage when changing behavior or safety checks. Never assume a
successful build validates interactive behavior on all platforms.

By submitting a contribution, you confirm that you have the right to contribute
it and license it under the repository's [LICENSE](LICENSE). You retain your
copyright. Identify any third-party code or assets and preserve their licenses;
do not contribute material whose terms conflict with this project.
