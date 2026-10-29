# VijiDeck

A native desktop utility app written entirely in Rust, with a system overview, port manager, and disk cleanup tool. No Node.js, JavaScript, browser UI, server, or webview.

## Run

Install the stable [Rust toolchain](https://rustup.rs/), then:

```sh
cargo run --release
```

Build a standalone executable with `cargo build --release`. It is in `target/release/vijideck.exe` on Windows and `target/release/vijideck` on macOS/Linux. See the packaging instructions below for installable distributions.

Native build requirements:

- **Windows:** Visual Studio Build Tools with Desktop development with C++ and the Windows SDK.
- **macOS:** Xcode Command Line Tools (`xcode-select --install`), including Clang.
- **Linux (Ubuntu/Debian):** `sudo apt-get install build-essential pkg-config libclang-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev`. A desktop session and OpenGL support are required to run the UI.

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
| Linux x64 (Debian/Ubuntu) | `sudo apt-get install dpkg-dev desktop-file-utils` | `.deb` with application-menu entry and app icon, plus a portable `.tar.gz` containing the icon asset. Install with `sudo apt install ./vijideck-<version>-linux-x86_64.deb`; remove with `sudo apt remove vijideck`. Extract the portable archive and run `./vijideck`; it requires the same desktop libraries as the Debian package. |

CI builds Windows x64, macOS Intel and Apple Silicon, and Linux x64 packages on every push, pull request, and manual run. Linux builds use Ubuntu 22.04 as the baseline; the Debian package calculates versioned linked-library dependencies and declares dynamically loaded desktop libraries. Other Linux distributions can use the portable archive when compatible libraries are available; RPM and AppImage are not supplied. The Linux packager also supports native arm64 builds, but these are not covered by the CI matrix.

To prepare a release, update `Cargo.toml` and `Cargo.lock` together, commit, then push a matching tag such as `v0.1.0`. After all platform checks and packages succeed, CI creates a **draft GitHub Release** with the packages and combined `SHA256SUMS.txt`. Review the draft before publishing. A tag that differs from the Cargo version fails packaging. Untagged builds remain available as workflow artifacts.

Packages currently have no trusted distribution signature. macOS bundles use an ad-hoc signature, which does not provide Developer ID trust or notarization. Windows SmartScreen and macOS Gatekeeper may warn or block downloaded packages. Public distribution with trusted installation requires Windows code-signing credentials and an Apple Developer ID/notarization setup. Auto-updates are not configured. Checksums verify file integrity, not publisher identity. The macOS bundle identifier is `app.vijideck.desktop`; choose an identifier under your control before signing a public release.

## System overview

### Background monitor and tray

The **Widget** button switches the main window to a small, semi-transparent, always-on-top CPU/RAM/GPU widget with rounded corners. Three circular gauges show current usage. Drag the widget to move it. Its controls open the full app, close VijiDeck, or minimize to the tray when available. Window size and widget position are remembered while the app is running. Missing GPU readings show a dash, and delayed telemetry is labeled.

On Windows, VijiDeck also appears in the notification area (including its overflow menu). Click the tray icon to open the app, or right-click for **Open VijiDeck**, **Show widget**, **Hide widget**, and **Quit VijiDeck**. Closing the main window or using the widget close button stops the app. If tray initialization fails, the minimize button is hidden so the app remains accessible.

Monitoring runs in the signed-in desktop session while the app is running. It does not install an operating-system service or start automatically at login. macOS/Linux support the floating widget but currently have no tray integration. Desktop transparency and always-on-top behavior depend on the window manager/compositor.

For a widget screenshot, use the existing preview feature with `VIJIDECK_PREVIEW_VIEW=widget` and `VIJIDECK_SCREENSHOT_TO=<output.png>` when running `cargo run --features ui-preview`.

The app opens on **System overview**. The first icon in the tool rail returns to it.

- Large circular indicators show total CPU utilization, usable memory in use, and live GPU utilization, with only the GPU model and memory capacity beneath the GPU gauge (the active adapter with the most dedicated memory). If neither GPU details nor utilization readings are available after inventory completes, the third indicator falls back to mounted storage capacity. A known GPU with unavailable utilization keeps its details and an unavailable reading. CPU is normalized to 0–100% for the entire machine. Mounted storage capacity also remains in the storage section below.
- On Windows, GPU sampling uses native GPU Engine performance counters, sums process utilization per engine, then shows the busiest engine across graphics adapters. This follows the [busiest-engine approach described by Microsoft](https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/). Sampling runs on the existing background worker and requires supported drivers. Missing or invalid counters show an unavailable state rather than zero usage.
- Background workers collect real telemetry approximately once per second, with CPU history for up to 60 samples, individual logical-processor bars, uptime, swap, supported temperature sensors, and network receive/send rates. Hover a processor bar for its utilization.
- Processor details show the model, physical cores, logical processors, and OS-reported frequency. Mounted volumes show their name, mount point, storage type, filesystem, capacity, and free space.
- On Windows, a separate, hidden PowerShell process makes fixed, read-only CIM queries for computer model, motherboard, graphics adapters and drivers, RAM modules, physical drives, firmware, displays, audio devices, physical network adapters, battery, keyboards, and pointing devices. Inventory is collected at launch with a 25-second timeout. These queries do not elevate permissions. Restricted or missing details are marked unavailable or not reported.
- macOS/Linux show portable telemetry and kernel/architecture details; extended device inventory and GPU utilization are currently Windows-only. VRAM usage and fan speeds are not collected. Temperature availability depends on OS and drivers. Memory totals represent usable memory. Network rates include virtual and loopback interfaces and do not measure internet connection speed.
- The overview scrolls to the complete inventory and network list, and adapts its processor/storage layout at the minimum window size. No hardware data is sent anywhere.

## Developer environment

Choose **</>** in the tool rail to inventory common languages, runtimes, package managers, and developer tools. The scan runs in the background; search filters names, versions, and paths, and Refresh rescans the current environment. Expand a technology group to see its installations and related tools, such as Python and pip. Each entry shows its path, reported version, and whether its directory is on the PATH inherited by VijiDeck. The app searches the user directory and platform-specific installation trees to a depth of seven directories, with a limit of 50,000 directories per root. It skips links and common project dependencies and caches. Search limits and inaccessible directories mean results are not exhaustive. Restart VijiDeck to pick up environment changes made after launch. Version checks execute the discovered tools locally with a version argument and a timeout. Windows command and batch wrappers are listed but their versions are unavailable. Some tools or versions may be inaccessible or absent from the supported list.
PATH actions change the directory containing an executable, so they affect every tool in that directory. On Windows they edit the current user's persistent PATH, preserving unexpanded variables; system PATH entries are not edited. On macOS and Linux they add or remove VijiDeck-managed entries in `~/.profile`. Login shells must read that file; graphical applications and other shell configurations may use a different environment. Restart applications or open a new login shell to pick up changes. The status shown in VijiDeck reflects the PATH it inherited at startup.
## Port manager

- Enumerates real TCP/UDP sockets over IPv4/IPv6 using native OS APIs through `netstat2`.
- Defaults to listening TCP and bound UDP sockets; uncheck **Listening / bound only** to include active and closing TCP connections.
- Search port, PID, process name, executable path, arguments, working directory, user, local/remote address, protocol, or socket state. Space-separated terms are combined with AND, case-insensitively. Search matches substrings.
- Sort by port, protocol, PID, or process using table headings. Select any row to inspect it. The inspector is resizable.
- Minimal HUD interface with a compact tool rail, cyan selection accents, distinct TCP/UDP badges, and an adaptive socket table. The inspector groups details into **Overview**, **Process**, and **Sockets** tabs, with process controls anchored below the scrollable details.
- Inspect process name, PID, status, executable, arguments, working directory, user, parent PID/name, start time, uptime, CPU, resident/virtual memory, lifetime disk read/write totals, and all socket entries owned by that process. Selectable text and a copy-details button are provided.
- Refresh manually or every two seconds. Socket collection and process operations run on a background worker.
- **Terminate** requests SIGTERM where supported. **Force kill** forcibly ends the process. Windows exposes Force kill only. Both actions require confirmation inside the app and affect the entire process, not just one port.
- Before sending a signal, the backend refreshes process identity and verifies the selected socket still belongs to that PID. It rejects changed/missing identities and protects PID 0–4 and the app itself. There is still a small OS-level race between verification and signaling; this is not a security boundary.
- `Ctrl+F` / `Cmd+F` focuses search; `Ctrl+R` / `Cmd+R` refreshes.

## Disk cleanup

Choose the **disk icon** in the tool rail. The opening screen shows mounted disks as square tiles with their names, storage types, capacity bars, and free space. Select a tile to enable **Scan [disk]**. The action stays disabled until you select a disk.

- Scanning runs off the UI thread and streams results as individual folders finish measuring. The elapsed counter includes hundredths of a second and stops when scanning finishes. Live counters show directories checked, files measured, complete matches, and recognized file bytes. **Stop scan** keeps completed results; incomplete measurements are discarded. No percentage or ETA is estimated without knowing the total work.
- Detects Cargo `target` (beside `Cargo.toml`), Maven `target` (beside `pom.xml`), .NET `bin`/`obj` (beside a project file), Node.js `node_modules`, Next.js `.next`, Nuxt `.nuxt`/`.output`, SvelteKit `.svelte-kit`, Parcel `.parcel-cache`, Turborepo `.turbo`, Gradle `.gradle`/`build`, CMake `build` (with project and cache markers), and Python `__pycache__`, `.pytest_cache`, `.mypy_cache`, and `.ruff_cache`. Node folders require a sibling `package.json`; Gradle folders require a sibling build script.
- Results lead with the folder and project path, source, size, and modified date on wide windows. Select a row or its checkbox; click **…** or double-click for the folder's full path, dates, type, and file count. The sort menu offers size, modified/created date, source, type, and path order. The search box matches space-separated terms with AND, case-insensitively; `Ctrl+F` / `Cmd+F` focuses it. Type tabs and a **Filters** popover refine source and minimum size; active filters have removable chips. Row context menus copy paths.
- Nested matches are counted once. Links, Windows reparse points, source control metadata, known system directories, other mounted disks, and unreadable paths are skipped. A candidate containing a link, mount, special file, or read error is marked incomplete and cannot be selected.
- Select folders with the table's header checkbox or row checkboxes, then choose **Review deletion** in the fixed footer. The footer shows total selected size and includes selections hidden by filters in the review. Deletion is permanent and bypasses the Recycle Bin/Trash. Close applications and builds using these folders first. Dependencies may require another download and generated output must be rebuilt.
- Before deleting, the backend checks the root boundary, ancestors, and a fresh inventory against the scanned file types, lengths, and modification/creation timestamps. Changed contents are rejected. Files are removed individually, followed by empty directories; no recursive delete command is used. Locked files or changes during deletion can leave partial results, which are reported. Rescan after any deletion attempt.

Detection is limited to recognized development artifacts; this is not a general browser, operating-system, or arbitrary `cache`/`build`/`dist` folder cleaner. Sizes are logical bytes, not allocated disk space: compression, sparse files, and hard links mean actual space recovered can differ. Checks reduce accidental deletion but are not an OS-level security boundary against concurrent adversarial filesystem changes; file contents are not hashed. The app does not elevate permissions or automatically delete anything.

### Scan performance

The scanner uses a directory-at-a-time work queue with a bounded pool: up to eight workers on SSD/unknown storage, limited by available CPU parallelism, and one worker on recognized rotational disks to avoid excessive seeks. Discovery skips ordinary files using directory-entry types, retains only relevant project marker names, and prunes recognized generated folders from further discovery. Measurement consumes directory-entry metadata instead of making an extra filesystem metadata request for each file. [Rust documents that directory-entry metadata is cached on Windows and file types are usually available without extra system calls.](https://doc.rust-lang.org/std/fs/struct.DirEntry.html) Directory-level parallelism overlaps independent reads; it does not accelerate enumeration of a single flat directory in the same way. [See the parallel traversal discussion in jwalk.](https://docs.rs/jwalk/latest/jwalk/enum.Parallelism.html)

File inventories use relative-path hash maps with shared ownership, reducing path storage, insertion cost, and copying while preserving deletion validation. Progress callbacks are coalesced to roughly ten per second; UI event draining has a frame time budget. Search text is normalized once per candidate, query terms once per edit, and matching/sorting is cached until data or filters change. Only visible table rows are drawn.

A local Windows SSD release-build measurement on 48 projects with 49,152 data files (plus project marker files), using four warm-cache passes, decreased from **4.46–5.62 seconds** before these changes to **0.043–0.054 seconds** afterward. This synthetic fixture is not a whole-disk guarantee; permissions, storage type, cold caches, and directory layout affect results. Rerun the optimized benchmark with:

```sh
cargo test --locked --release cleanup::tests::benchmark_scan -- --ignored --nocapture
```

The benchmark creates its reusable fixture under `target/cleanup-benchmark-data` and verifies folder counts and byte totals. Fixture creation is excluded from timing.

## Platform and data limitations

Socket/process visibility depends on OS permissions. Restricted fields are marked unavailable; socket enumeration errors are shown alongside the scan time. Running as administrator/root may expose more data and allow stopping processes owned by other users. The app never elevates itself automatically.

CPU values need two samples and are per-core, so multithreaded processes can exceed 100%. Each socket owner produces a row; unknown owners remain visible but cannot be stopped. UDP has no TCP connection state. A wildcard bind means all interfaces, not necessarily internet exposure. Remote names, process environment variables, open files, service control, process-tree termination, and firewall editing are not collected or implemented. No process data is sent anywhere.

Process identity uses PID plus the OS-reported start timestamp, whose precision varies by platform. A successful signal request does not guarantee termination; the next scan shows whether sockets remain. A service supervisor may restart a killed process.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests discover a real loopback listener and its owner, verify search, reject self/system targets, reject stale identities and changed socket ownership, then force-kill only a disposable test child and check its port is released. The ignored child-listener test is an internal fixture invoked by its parent test. Cleanup tests use disposable temporary directories to check detection, byte totals, nested matches, cancellation, root boundaries, stale contents, selected deletion, mount protection, and Windows junction or Unix link rejection.

Cleanup visual previews use `VIJIDECK_PREVIEW_VIEW=cleanup`, `cleanup-disks`, `cleanup-scanning`, `cleanup-filters`, or `cleanup-confirm`, with `VIJIDECK_PREVIEW_CLEANUP_ROOT` set to a small fixture directory for result views. Result views scan only that directory and never trigger deletion. The scanning preview simulates an in-progress timer over those results. They also support `VIJIDECK_PREVIEW_COMPACT=1`.

CI runs checks, tests, and native packaging on Windows, macOS (Intel and Apple Silicon), and Linux and uploads installable packages. Local verification on one OS does not validate the other platforms.

For visual QA with real local socket data, set `VIJIDECK_SCREENSHOT_TO` to a `.png` file path and run `cargo run --features ui-preview`. This opt-in mode preloads a scan, selects an accessible process, lets the native layout settle, captures the window, and exits. Set `VIJIDECK_PREVIEW_COMPACT=1` for the 900 × 580 minimum window size. `VIJIDECK_PREVIEW_VIEW` accepts `process`, `sockets`, `empty`, or `confirm` to inspect those UI states; the confirmation preview does not send any signal. Use `overview` for the default dashboard or `overview-hardware` for its scrolled inventory; these wait for inventory and two telemetry samples before capturing (up to 28 seconds). Normal launches load data in the background.

Sources: [eframe](https://docs.rs/eframe/0.33.3/eframe/), [netstat2](https://docs.rs/netstat2/0.11.2/netstat2/), [sysinfo](https://docs.rs/sysinfo/0.37.2/sysinfo/).

## System logs

Choose **Logs** in the tool rail to open recent operating system events.

- Windows opens Application logs by default; choose System or Security when needed. Linux opens the current user's journal by default, with system and kernel journals available on systemd. macOS opens unified logs by default, with kernel events available separately.
- Restricted sources show an access prompt with **Grant log access** and **Show accessible logs**. Windows opens a separate, logs-only administrator window after the user accepts the UAC prompt. Linux uses PolicyKit (`pkexec`); macOS uses its administrator authentication dialog for that log query. Cancelling keeps the regular log view available. The app never requests elevation automatically.
- Choose the last 15 minutes, hour, or 24 hours, then refresh. Displays up to 2,000 newest events. Search matches space-separated terms with AND, case-insensitively, across loaded event fields; severity filters also apply to loaded events. Search does not query the entire log history.
- Choose **Calendar range…** to select start and end dates from a calendar and choose each hour. The selected start hour begins at `:00`; the end hour includes events through `:59`. Apply the range to query those local times. **Clear range** removes the calendar filter and returns to the last quick time preset. Choosing any quick time preset also removes the calendar filter.
- Select an event for its full message and raw fields; **Copy event** copies the full event. `Ctrl+F` / `Cmd+F` focuses search and `Ctrl+R` / `Cmd+R` refreshes.
- Reads run in a background worker with a 30-second timeout and 32 MiB output limit; an explicitly approved access request allows up to 120 seconds for authentication. If a busy macOS log exceeds the output limit, choose a shorter time range. The app never modifies logs. macOS private values may be redacted. Empty results can reflect restricted visibility.

Native macOS and Linux runtime verification requires those operating systems; the Windows reader and platform record parsers can be tested locally with `cargo test`. To explicitly query real host logs, run `cargo test logs::tests::reads_host_logs -- --ignored --nocapture`.
