# Development and packaging

[Back to VijiDeck](../README.md)

## Build installers and packages

On each target OS, install Rust and the native build requirements above, then run from the repository root:

```sh
cargo xtask package
```

The `xtask` command is a separate Rust tool with no crate dependencies; Python is not required. It coordinates native tools such as Inno Setup, `hdiutil`, and `dpkg-deb`. It builds with the lockfile and an explicit native Rust target, reads the version from `Cargo.toml`, and writes packages and SHA-256 checksums under `dist/<version>-<os>-<architecture>/`. `cargo xtask package --skip-build` reuses the executable previously built by this command (or `cargo build --locked --release --target <rust-host-triple>`). Builds use the root `target/` directory, or `CARGO_TARGET_DIR` if set. Cross-compilation is not supported by the packager.

| Platform | Additional packaging tools | Output and installation |
| --- | --- | --- |
| Windows x64 | [Inno Setup 6](https://jrsoftware.org/isdl.php) (CI uses 6.4.0); set `ISCC` to `ISCC.exe` if installed outside the default locations | `*-setup.exe` wizard and portable `.zip`. The installer, running window, Start menu shortcut, optional desktop shortcut, and uninstaller use the app icon. Choose **Install for me** (the default, under `%LOCALAPPDATA%\Programs`, without elevation) or **Install for everyone** (under Program Files, with a Windows UAC prompt). The wizard also offers an installation folder, Start menu folder, optional desktop shortcut, and launch after installation. Remove through Windows Settings → Apps. Rerunning a newer installer upgrades the same installation in the selected scope. Silent installs can use `/CURRENTUSER` or `/ALLUSERS`. |
| macOS Apple Silicon / Intel | Xcode Command Line Tools (`hdiutil` and `codesign` are built in) | Architecture-specific `.dmg` containing `VijiDeck.app` with its app icon and an Applications shortcut. Open the DMG and drag the app into Applications; eject the DMG. Remove the app from Applications to uninstall. |
| Linux x64 (Debian/Ubuntu) | `sudo apt-get install dpkg-dev desktop-file-utils appstream` | `.deb` with application-menu entry and hicolor app icons (16–512 px, from `packaging/linux/icons`), plus a portable `.tar.gz` containing the same icon tree. Install with `sudo apt install ./vijideck-<version>-linux-x86_64.deb`; remove with `sudo apt remove vijideck`. Extract the portable archive and run `./vijideck`; it requires the same desktop libraries as the Debian package. |

CI builds Windows x64, macOS Intel and Apple Silicon, and Linux x64 packages on every push, pull request, and manual run. Linux builds use Ubuntu 22.04 as the baseline; the Debian package calculates versioned linked-library dependencies and declares dynamically loaded desktop libraries. Other Linux distributions can use the portable archive when compatible libraries are available; RPM and AppImage are not supplied. The Linux packager also supports native arm64 builds, but these are not covered by the CI matrix. The `.deb` requires at least the glibc version of the machine that built it, so a package built locally on a newer distribution (for example Ubuntu 26.04) will not install on Debian 12/13; use the CI artifacts, or build on the oldest Debian/Ubuntu release you need to support.

To prepare a release, update `Cargo.toml` and `Cargo.lock` together, commit, then push a matching tag such as `v0.1.4`. After all platform checks and packages succeed, CI creates a **draft GitHub Release** with the packages and combined `SHA256SUMS.txt`. Review the draft before publishing. A tag that differs from the Cargo version fails packaging. Untagged builds remain available as workflow artifacts.

Packages currently have no trusted distribution signature. macOS bundles use an ad-hoc signature, which does not provide Developer ID trust or notarization. Windows SmartScreen and macOS Gatekeeper may warn or block downloaded packages. Public distribution with trusted installation requires Windows code-signing credentials and an Apple Developer ID/notarization setup. Auto-updates are not configured. Checksums verify file integrity, not publisher identity. The macOS bundle identifier is `app.vijideck.desktop`; choose an identifier under your control before signing a public release.

## Checks and visual QA

See [the architecture and maintenance audit](architecture.md) for module responsibilities, refactoring decisions, and remaining limitations.

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

Tests discover a real loopback listener and its owner, verify search, reject self/system targets, reject stale identities and changed socket ownership, then force-kill only a disposable test child and check its port is released. The ignored child-listener test is an internal fixture invoked by its parent test. Cleanup tests use disposable temporary directories to check detection, byte totals, nested matches, cancellation, root boundaries, stale contents, selected deletion, mount protection, and Windows junction or Unix link rejection.

Cleanup visual previews use `VIJIDECK_PREVIEW_VIEW=cleanup`, `cleanup-disks`, `cleanup-scanning`, `cleanup-filters`, or `cleanup-confirm`, with `VIJIDECK_PREVIEW_CLEANUP_ROOT` set to a small fixture directory for result views. Result views scan only that directory and never trigger deletion. The scanning preview simulates an in-progress timer over those results. They also support `VIJIDECK_PREVIEW_COMPACT=1`.

CI runs checks, tests, and native packaging on Windows, macOS (Intel and Apple Silicon), and Linux and uploads installable packages. Local verification on one OS does not validate the other platforms.

For visual QA with real local socket data, set `VIJIDECK_SCREENSHOT_TO` to a `.png` file path and run `cargo run --features ui-preview`. This opt-in mode preloads a scan, selects an accessible process, lets the native layout settle, captures the window, and exits. Set `VIJIDECK_PREVIEW_COMPACT=1` for the 900 × 580 minimum window size. `VIJIDECK_PREVIEW_VIEW` accepts `process`, `sockets`, `empty`, or `confirm` to inspect those UI states; the confirmation preview does not send any signal. Use `overview` for the default dashboard or `overview-hardware` for its scrolled inventory; these wait for inventory and two telemetry samples before capturing (up to 28 seconds). Normal launches load data in the background.

Sources: [eframe](https://docs.rs/eframe/0.33.3/eframe/), [netstat2](https://docs.rs/netstat2/0.11.2/netstat2/), [sysinfo](https://docs.rs/sysinfo/0.37.2/sysinfo/).


## License metadata

Linux packages install AppStream metadata with a custom/proprietary license reference
and a link to the full terms. Software centers may label this as proprietary or
special; they do not all display custom license names. The metadata file itself
is MIT-licensed for catalog aggregation; the application retains its custom license.
Windows setup displays the full license and carries publisher/copyright metadata.
macOS bundles include a human-readable copyright/license notice in Info.plist and
the full terms in Contents/Resources/LICENSE. Rebuild packages to apply these changes.
