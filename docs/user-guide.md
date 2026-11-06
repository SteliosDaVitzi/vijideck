# User guide

[Back to VijiDeck](../README.md)

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

## Process groups and mounted volumes

On Windows, Processes groups Explorer and its descendants separately from system and other processes. On Linux and macOS, the first section is **User processes**, based on the owner of the running VijiDeck process; it includes that user's applications and background processes. Processes owned by other accounts appear under **System**, and unavailable ownership appears under **Other**. Name, PID, CPU, Memory, and Status headers toggle sorting within each section. CPU and Memory sort application groups by their displayed totals, and expanded children remain beneath their group. Search preserves process ancestry and ownership classification.

Overview and Disk cleanup share the same volume filter. Linux hides boot/EFI, Docker/Podman/container runtime mounts, Snap images, and virtual system filesystems; ordinary root, home, data, and removable volumes remain visible. Cleanup also skips recognized container storage trees during discovery and cached-result validation. Its mount-boundary checks continue to use the complete mount list. On macOS, the writable Data volume is preferred over the paired system root when present; internal APFS service volumes are hidden and external volumes remain visible. Windows drive letters and mounted data volumes are retained.

The regression tests exercise Windows, macOS, and Linux grouping and volume rules on every CI runner. Native CI builds cover Windows x64, macOS Intel/Apple Silicon, and Linux x64; interactive desktop behavior still requires validation on each operating system.

## Developer environment

Choose **</>** in the tool rail to inventory common languages, runtimes, package managers, and developer tools. The scan runs in the background; search filters names, versions, and paths, and Refresh rescans the current environment. Expand a technology group to see its installations and related tools, such as Python and pip. Each entry shows its path, reported version, and whether its directory is on the PATH inherited by VijiDeck. The app searches the user directory and platform-specific installation trees to a depth of seven directories, with a limit of 50,000 directories per root. It skips links and common project dependencies and caches. Search limits and inaccessible directories mean results are not exhaustive. Restart VijiDeck to pick up environment changes made after launch. Version checks execute the discovered tools locally with a version argument and a timeout. Windows command and batch wrappers are listed but their versions are unavailable. Some tools or versions may be inaccessible or absent from the supported list.
PATH actions change the directory containing an executable, so they affect every tool in that directory. On Windows they edit the current user's persistent PATH, preserving unexpanded variables; system PATH entries are not edited. On macOS and Linux they add or remove VijiDeck-managed entries in `~/.profile`. Login shells must read that file; graphical applications and other shell configurations may use a different environment. Restart applications or open a new login shell to pick up changes. The status shown in VijiDeck reflects the PATH it inherited at startup.

Installations are grouped by likely origin: standalone, managed, bundled, or unknown. VijiDeck checks known application and manager directories and, on Windows, installed-app, PythonCore, and Node.js registry locations. This is a best-effort classification, not proof of ownership; some installers provide no location, and other operating systems currently use path evidence only. Hover an origin or version pill to see its evidence. Amber version pills indicate a likely bundled runtime, and the same version may have separate pills for different origins.
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

## Services

The **Services** rail icon opens the local service manager. Search matches names,
identifiers, descriptions, state, startup mode, and configuration/command. State
and startup filters work together. Select a service to inspect its identifier,
scope, process ID (when available), and configuration. Hover or focus the icon
buttons for their action names. Auto refresh runs every ten seconds; the refresh
icon retries discovery immediately.

- **Windows:** Lists installed services, including stopped and disabled services.
  Start, stop, restart, pause, and resume depend on each service's capabilities.
  Startup can be Automatic, Manual, or Disabled. Enable sets Automatic; Manual is
  available in the inspector.
- **Linux:** Requires systemd and `systemctl`. System and Current user query their
  respective managers, combining installed service files with loaded/transient
  units. Static, masked, generated, and template units retain their native startup
  states; unsupported startup actions stay disabled. Uninstantiated templates
  cannot be started. Pause/resume is unavailable.
- **macOS:** System lists launch daemons; Current user lists the signed-in GUI
  session's launch agents. Discovery includes standard Apple, local, and user
  plist directories plus jobs loaded into the chosen domain. Start bootstraps an
  unloaded configured job and kickstarts it. Stop unloads it from that domain;
  it may load again at the next login or boot. Restart uses launchd's kickstart
  restart. Enable/disable changes launchd's persistent override. Pause/resume is
  unavailable. A stopped on-demand job is normal. If macOS changes its diagnostic
  inventory format, discovery reports an error and actions remain unavailable.

Startup changes do not immediately start or stop a service. Stop, restart, pause,
and startup changes show a confirmation with the exact service and scope.
VijiDeck re-reads the service before dispatching an action and rejects stale
selections. Actions run in the background and refresh inventory afterward, even
when the action fails. A timeout can leave completion uncertain; refresh and
review the current state before retrying.

System changes often require administrator/root privileges. Windows users can
launch VijiDeck as administrator. Linux uses the current desktop's systemd
authorization policy without a terminal password prompt. macOS system changes
require a process with sufficient privileges. VijiDeck reports access errors;
it does not silently elevate or force operations against protected services.
Other applications and operating-system features can depend on these services.

## System logs

Choose **Logs** in the tool rail to open recent operating system events.

- Windows opens Application logs by default; choose System or Security when needed. Linux opens the current user's journal by default, with system and kernel journals available on systemd. macOS opens unified logs by default, with kernel events available separately.
- Restricted sources show an access prompt with **Grant log access** and **Show accessible logs**. Windows opens a separate, logs-only administrator window after the user accepts the UAC prompt. Linux uses PolicyKit (`pkexec`); macOS uses its administrator authentication dialog for that log query. Cancelling keeps the regular log view available. The app never requests elevation automatically.
- Choose the last 15 minutes, hour, or 24 hours, then refresh. Displays up to 2,000 newest events. Search matches space-separated terms with AND, case-insensitively, across loaded event fields; severity filters also apply to loaded events. Search does not query the entire log history.
- Choose **Calendar range…** to select start and end dates from a calendar and choose each hour. The selected start hour begins at `:00`; the end hour includes events through `:59`. Apply the range to query those local times. **Clear range** removes the calendar filter and returns to the last quick time preset. Choosing any quick time preset also removes the calendar filter.
- Select an event for its full message and raw fields; **Copy event** copies the full event. `Ctrl+F` / `Cmd+F` focuses search and `Ctrl+R` / `Cmd+R` refreshes.
- Reads run in a background worker with a 30-second timeout and 32 MiB output limit; an explicitly approved access request allows up to 120 seconds for authentication. If a busy macOS log exceeds the output limit, choose a shorter time range. The app never modifies logs. macOS private values may be redacted. Empty results can reflect restricted visibility.

Native macOS and Linux runtime verification requires those operating systems; the Windows reader and platform record parsers can be tested locally with `cargo test`. To explicitly query real host logs, run `cargo test logs::tests::reads_host_logs -- --ignored --nocapture`.

## Local preferences

VijiDeck stores port-manager auto-refresh, listening-only filter, protocol, and sorting preferences in a local SQLite database. On Windows it is located at `%LOCALAPPDATA%\VijiDeck\vijideck.sqlite3`; on macOS at `~/Library/Application Support/VijiDeck/vijideck.sqlite3`; and on Linux at `$XDG_DATA_HOME/vijideck/vijideck.sqlite3` (or `~/.local/share/vijideck/vijideck.sqlite3`). The database is created on first launch. If it cannot be opened, the app uses defaults for that session. Screenshot previews do not use the database.

Developer tool locations and the most recently completed cleanup disk scan are cached in SQLite. When the corresponding view is first opened, cached tool paths are checked and versions refreshed; cached cleanup folders are reclassified and remeasured before they appear. A background sync then searches for new results, with a syncing indicator in each view. The first cleanup scan still requires choosing a disk; subsequent sessions rescan that disk when Cleanup is first opened. The dashboard and floating widget do not start port, developer-tool, or cleanup scans. Port scanning starts when Ports is opened, including when auto refresh is paused. Cancelled cleanup scans do not replace the last completed cache. Cleanup selections remain disabled during scanning, and deletion still performs its own fresh validation.

### Docker

Horizontal **Containers**, **Images**, and **Volumes** tabs show inventories from the current Docker context. Images lists repository, tag, full image ID, creation date, and size, including untagged images. An image with several tags can appear in several rows. Volumes lists name, driver, scope, and the daemon's mount point. Images and Volumes each have independent search and auto-refresh controls; Refresh retries the active tab. Switching tabs preserves container filters and Compose expansion and closes any container log stream.

The Containers tab lists running and stopped containers from the Docker CLI's current
context/environment. Docker must be installed on PATH, with a running daemon that
your account can access. The view refreshes every three seconds while open; auto
refresh can be paused, and Refresh retries connection errors.

Compose projects appear as expandable rows with their containers underneath.
Search by project, service, container name, image, status, ID, or port, or select
Running only. Start and Stop work on individual containers or eligible containers
in a whole project. Project actions include all its containers even when a search
hides some children. Start operates on existing containers; it does not recreate
containers or apply Compose dependency ordering.

Delete asks for confirmation and removes the listed containers, including running
ones. Container writable data is lost; images, volumes, and networks are kept.
After deleting a Compose project's containers, use `docker compose up` from the
project directory to recreate them. Command errors are shown in the tool and the
inventory is refreshed after actions, including partially successful actions.

Container rows include a **Logs** button. It opens a resizable bottom drawer with
live stdout and stderr, starting with the last 200 timestamped lines. Toggle
Follow to read older output, Clear to clear the visible buffer, or Close to stop
the stream. History is capped at 2,000 lines; very long lines are split into
bounded chunks. Switching containers or leaving the Docker tool closes the
previous stream. Click Logs again to reconnect an ended stream.

Background inventory refreshes preserve the table, expansion state, scroll
position and enabled controls. Start, Stop and Delete temporarily disable only
mutation actions while their command runs; an action requested during an inventory
scan is queued until that scan completes.

Container actions use egui's bundled play, stop and trash symbols and a classic `>_` terminal icon with
hover tooltips. Compose rows use egui's built-in disclosure arrow. The terminal
button is enabled for running containers and opens `docker exec -it <id> sh` in
a desktop terminal (Konsole/system terminal on Linux, Windows Terminal or
PowerShell on Windows, Terminal on macOS). Exit the shell to leave the session.
Containers without `sh` report that error in the terminal.
