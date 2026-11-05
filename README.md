# VijiDeck

A native desktop toolkit for keeping an eye on your machine and managing your development environment. Built in Rust with egui/eframe for Windows, macOS, and Linux.

## What it does

- **System overview:** CPU and memory usage, storage, network activity, process groups, and a floating monitoring widget. Windows also provides GPU usage, extended hardware inventory, and a tray menu.
- **Ports and processes:** Search TCP/UDP sockets, inspect their owning processes, and terminate a process after confirmation.
- **Developer environment:** Find installed languages and tools, inspect versions and installation paths, and manage PATH entries and environment variables.
- **Disk cleanup:** Find recognized development dependencies, caches, and build output, then review selected folders before permanently deleting them.
- **Docker:** Browse containers, Compose projects, images, and volumes. Start, stop, or remove containers, follow logs, and open a container shell through your desktop terminal.
- **System logs:** Browse, filter, and inspect Windows event logs, Linux journals, or macOS unified logs.

VijiDeck runs as a native application, without a browser UI, webview, or application server. Preferences and discovery caches are stored locally in SQLite. Docker uses your configured Docker context, which may point to a remote daemon. See the [user guide](docs/user-guide.md) for behavior, permissions, data storage, and limitations.

## Get started

Download available packages from [GitHub Releases](https://github.com/SteliosDaVitzi/vijideck/releases), or build from source below. CI builds are also available as artifacts in [Actions](https://github.com/SteliosDaVitzi/vijideck/actions).

| Platform | Packages | Notes |
| --- | --- | --- |
| Windows x64 | Installer and portable ZIP | Tray integration and extended hardware/GPU monitoring |
| macOS Intel / Apple Silicon | Architecture-specific DMG | Drag VijiDeck into Applications |
| Linux x64 | Debian package and portable tarball | CI uses Ubuntu 22.04; compatible desktop libraries are required |

Packages are currently unsigned by a trusted publisher; macOS uses an ad-hoc signature. Windows SmartScreen or macOS Gatekeeper may warn or block installation. Automatic updates are not available. Platform builds run in CI, but interactive behavior needs testing on each OS.

### Build from source

Install stable [Rust](https://rustup.rs/) and the native dependencies for your OS:

- **Windows:** Visual Studio Build Tools with Desktop development with C++ and the Windows SDK.
- **macOS:** Xcode Command Line Tools (`xcode-select --install`).
- **Ubuntu/Debian:** `sudo apt-get install build-essential pkg-config libclang-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev`.

```sh
git clone https://github.com/SteliosDaVitzi/vijideck.git
cd vijideck
cargo run --locked --release
```

A desktop session and OpenGL support are required. `cargo build --locked --release` produces `target/release/vijideck` (`vijideck.exe` on Windows). Docker features additionally require the Docker CLI on PATH and access to its daemon; Linux journal features require systemd tooling.

## Before changing your system

Cleanup permanently deletes selected files, bypassing Trash/Recycle Bin. Stopping a process affects the entire process. Removing a container loses its writable data. PATH and environment edits affect future shell or application sessions. Review confirmations and close affected applications first. Read the [user guide](docs/user-guide.md) for safeguards and their limits.

## Documentation and contributions

- [User guide](docs/user-guide.md): features, platform differences, and local data.
- [Development and packaging](docs/development.md): checks, visual QA, installers, and releases.
- [Architecture](docs/architecture.md): module responsibilities and maintenance notes.
- [Contributing](CONTRIBUTING.md), [security reporting](SECURITY.md), and [release checklist](docs/releasing.md).

## License

VijiDeck is **source available**, under the custom [VijiDeck Personal and Internal Use License 1.0](LICENSE).

Personal use and internal professional use within companies are allowed, including work supporting a company's commercial activities. Selling VijiDeck, providing it as a hosted service, bundling it into a customer product, or using it to provide software services to third parties is prohibited without separate written permission. Free noncommercial redistribution and modifications are allowed under the license's conditions.

Because it restricts commercial exploitation, this is not an OSI-approved open source license. Third-party components retain their [own licenses](THIRD_PARTY_NOTICES.md).
