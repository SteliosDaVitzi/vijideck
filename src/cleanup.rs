//! Conservative, directory-based cleanup. Never follows symlinks or Windows reparse points.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    directory: bool,
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub source: &'static str,
    pub kind: &'static str,
    pub bytes: u64,
    pub files: usize,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub issue: Option<String>,
    pub search: String,
    manifest: Arc<HashMap<PathBuf, Stamp>>,
}

#[derive(Default)]
pub struct Scan {
    pub root: PathBuf,
    pub entries: Vec<Candidate>,
    pub visited: usize,
    pub measured_files: usize,
    pub workers: usize,
    pub elapsed: Duration,
    pub skipped: usize,
    pub warnings: Vec<String>,
    pub cancelled: bool,
}

#[derive(Clone, Default)]
pub struct Progress {
    pub directories: usize,
    pub files: usize,
    pub bytes: u64,
    pub found: usize,
    pub path: PathBuf,
}
pub enum ScanUpdate {
    Started(PathBuf, usize),
    Progress(Progress),
    Found(Candidate),
}

#[derive(Default)]
pub struct DeleteReport {
    pub removed: Vec<PathBuf>,
    pub bytes: u64,
    pub failures: Vec<String>,
}

fn linked(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn stamp(meta: &fs::Metadata) -> Stamp {
    Stamp {
        directory: meta.is_dir(),
        len: if meta.is_file() { meta.len() } else { 0 },
        modified: meta.modified().ok(),
        created: meta.created().ok(),
    }
}

fn has(names: &HashSet<String>, name: &str) -> bool {
    names.contains(name)
}

fn classify(path: &Path, siblings: &HashSet<String>) -> Option<(&'static str, &'static str)> {
    let name = path.file_name()?.to_str()?;
    let node = has(siblings, "package.json");
    match name {
        "target" if has(siblings, "Cargo.toml") => Some(("Rust / Cargo", "Build")),
        "target" if has(siblings, "pom.xml") => Some(("Maven", "Build")),
        "node_modules" if node => Some(("Node.js dependencies", "Cache")),
        ".next" if node => Some(("Next.js", "Build")),
        ".nuxt" | ".output" if node => Some(("Nuxt", "Build")),
        ".parcel-cache" if node => Some(("Parcel", "Cache")),
        ".turbo" if node => Some(("Turborepo", "Cache")),
        ".svelte-kit" if node => Some(("SvelteKit", "Build")),
        "build" if has(siblings, "CMakeLists.txt") && path.join("CMakeCache.txt").is_file() => {
            Some(("CMake", "Build"))
        }
        "bin" | "obj"
            if siblings.iter().any(|n| {
                n.ends_with(".csproj") || n.ends_with(".fsproj") || n.ends_with(".vbproj")
            }) =>
        {
            Some((".NET", "Build"))
        }
        "__pycache__" => Some(("Python bytecode", "Cache")),
        ".pytest_cache" => Some(("pytest", "Cache")),
        ".mypy_cache" => Some(("mypy", "Cache")),
        ".ruff_cache" => Some(("Ruff", "Cache")),
        ".gradle" if has(siblings, "build.gradle") || has(siblings, "build.gradle.kts") => {
            Some(("Gradle", "Cache"))
        }
        "build" if has(siblings, "build.gradle") || has(siblings, "build.gradle.kts") => {
            Some(("Gradle", "Build"))
        }
        _ => None,
    }
}

fn excluded(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        ".git" | ".hg" | ".svn" | "$recycle.bin" | "system volume information"
    ) {
        return true;
    }
    #[cfg(windows)]
    {
        name == "windows" && path.parent().is_some_and(|p| p.parent().is_none())
    }
    #[cfg(not(windows))]
    {
        matches!(path.to_str(), Some("/proc" | "/sys" | "/dev"))
    }
}

fn note(scan: &mut Scan, path: &Path, error: impl std::fmt::Display) {
    scan.skipped += 1;
    if scan.warnings.len() < 100 {
        scan.warnings.push(format!("{}: {error}", path.display()));
    }
}

/// Rebuild a cached candidate from current filesystem state before showing it.
pub fn validate_cached(root: &Path, path: &Path, cancel: &AtomicBool) -> Option<Candidate> {
    validate_path(root, path).ok()?;
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() || linked(&meta) {
        return None;
    }
    let parent = path.parent()?;
    let names: HashSet<String> = fs::read_dir(parent)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let (source, kind) = classify(path, &names)?;
    let mounts = sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter_map(|d| d.mount_point().canonicalize().ok())
        .filter(|p| p != root)
        .collect();
    let manifest = inventory(path, &mounts, cancel, &mut |_, _, _| {}).ok()?;
    let bytes = manifest.values().map(|s| s.len).sum();
    let files = manifest.values().filter(|s| !s.directory).count();
    let modified = manifest
        .values()
        .filter_map(|s| s.modified)
        .max()
        .or_else(|| meta.modified().ok());
    Some(Candidate {
        path: path.to_owned(),
        source,
        kind,
        bytes,
        files,
        modified,
        created: meta.created().ok(),
        issue: None,
        search: format!("{} {source} {kind}", path.display()).to_lowercase(),
        manifest: Arc::new(manifest),
    })
}

/// A bounded pool overlaps independent directory reads. Rotational disks use one
/// worker to avoid seek storms; SSD/unknown storage uses at most eight workers.
pub fn scan(root: &Path, cancel: &AtomicBool, update: impl FnMut(ScanUpdate) + Send) -> Scan {
    scan_with_workers(root, cancel, 0, update)
}

fn scan_with_workers(
    root: &Path,
    cancel: &AtomicBool,
    requested: usize,
    update: impl FnMut(ScanUpdate) + Send,
) -> Scan {
    let started = Instant::now();
    let mut result = Scan {
        root: root.to_path_buf(),
        ..Scan::default()
    };
    let root = match root.canonicalize() {
        Ok(root) if root.is_dir() => root,
        Ok(_) => {
            note(&mut result, root, "Choose a directory or disk");
            return result;
        }
        Err(e) => {
            note(&mut result, root, e);
            return result;
        }
    };
    result.root = root.clone();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter_map(|d| d.mount_point().canonicalize().ok().map(|p| (d, p)))
        .filter(|(_, p)| root.starts_with(p))
        .max_by_key(|(_, p)| p.components().count());
    let rotational = disk.is_some_and(|(d, _)| d.kind() == sysinfo::DiskKind::HDD);
    let workers = if requested > 0 {
        requested.clamp(1, 8)
    } else if rotational {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(2, usize::from)
            .clamp(1, 8)
    };
    result.workers = workers;
    let engine = Engine {
        queue: Mutex::new(Work {
            jobs: VecDeque::from([Job::Discover(root.clone())]),
            active: 0,
            finished: false,
        }),
        ready: Condvar::new(),
        result: Mutex::new(result),
        cancel,
        mounts: disks
            .list()
            .iter()
            .filter_map(|d| d.mount_point().canonicalize().ok())
            .filter(|p| p != &root)
            .collect(),
        reporter: Mutex::new((Instant::now(), update)),
        directories: AtomicUsize::new(0),
        files: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
        found: AtomicUsize::new(0),
    };
    (engine.reporter.lock().unwrap().1)(ScanUpdate::Started(root.clone(), workers));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let engine = &engine;
            scope.spawn(move || engine.run());
        }
    });
    engine.progress(&root, true);
    let mut result = engine.result.into_inner().unwrap();
    result.visited = engine.directories.load(Ordering::Relaxed);
    result.measured_files = engine.files.load(Ordering::Relaxed);
    result.cancelled = cancel.load(Ordering::Relaxed);
    result.elapsed = started.elapsed();
    result.entries.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

enum Job {
    Discover(PathBuf),
    Measure(Candidate),
}
struct Work {
    jobs: VecDeque<Job>,
    active: usize,
    finished: bool,
}
struct Engine<'a, F> {
    queue: Mutex<Work>,
    ready: Condvar,
    result: Mutex<Scan>,
    cancel: &'a AtomicBool,
    mounts: HashSet<PathBuf>,
    reporter: Mutex<(Instant, F)>,
    directories: AtomicUsize,
    files: AtomicUsize,
    bytes: AtomicU64,
    found: AtomicUsize,
}
impl<F: FnMut(ScanUpdate) + Send> Engine<'_, F> {
    fn progress(&self, path: &Path, force: bool) {
        // UI updates are coalesced globally, not emitted once per worker/file.
        if let Ok(mut reporter) = self.reporter.try_lock()
            && (force || reporter.0.elapsed() >= Duration::from_millis(100))
        {
            reporter.0 = Instant::now();
            (reporter.1)(ScanUpdate::Progress(Progress {
                directories: self.directories.load(Ordering::Relaxed),
                files: self.files.load(Ordering::Relaxed),
                bytes: self.bytes.load(Ordering::Relaxed),
                found: self.found.load(Ordering::Relaxed),
                path: path.to_path_buf(),
            }));
        }
    }
    fn run(&self) {
        loop {
            let job = {
                let mut work = self.queue.lock().unwrap();
                loop {
                    if self.cancel.load(Ordering::Relaxed) {
                        work.jobs.clear();
                    }
                    if let Some(job) = work.jobs.pop_back() {
                        work.active += 1;
                        break job;
                    }
                    if work.active == 0 || work.finished {
                        work.finished = true;
                        self.ready.notify_all();
                        return;
                    }
                    work = self.ready.wait(work).unwrap();
                }
            };
            let children = match job {
                Job::Discover(path) => self.discover(path),
                Job::Measure(candidate) => {
                    self.measure(candidate);
                    Vec::new()
                }
            };
            let mut work = self.queue.lock().unwrap();
            work.active -= 1;
            if !self.cancel.load(Ordering::Relaxed) {
                work.jobs.extend(children);
            }
            self.ready.notify_all();
        }
    }
    fn discover(&self, dir: PathBuf) -> Vec<Job> {
        if self.cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        self.directories.fetch_add(1, Ordering::Relaxed);
        self.progress(&dir, false);
        let mut part = Scan::default();
        let mut directories = Vec::new();
        let mut names = HashSet::new();
        let children = match fs::read_dir(&dir) {
            Ok(children) => children,
            Err(e) => {
                note(&mut part, &dir, e);
                self.merge(part);
                return Vec::new();
            }
        };
        for child in children {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let child = match child {
                Ok(c) => c,
                Err(e) => {
                    note(&mut part, &dir, e);
                    continue;
                }
            };
            let name = child.file_name();
            let name = name.to_string_lossy();
            // Only retain names used by detection. Avoid allocating paths or
            // requesting metadata for ordinary source files.
            if matches!(
                name.as_ref(),
                "Cargo.toml"
                    | "pom.xml"
                    | "package.json"
                    | "build.gradle"
                    | "build.gradle.kts"
                    | "CMakeLists.txt"
            ) || name.ends_with(".csproj")
                || name.ends_with(".fsproj")
                || name.ends_with(".vbproj")
            {
                names.insert(name.into_owned());
            }
            let kind = match child.file_type() {
                Ok(k) => k,
                Err(e) => {
                    note(&mut part, &child.path(), e);
                    continue;
                }
            };
            if kind.is_symlink() {
                part.skipped += 1;
                continue;
            }
            if !kind.is_dir() {
                continue;
            }
            let path = child.path();
            if excluded(&path) || self.mounts.contains(&path) {
                part.skipped += 1;
                continue;
            }
            match child.metadata() {
                Ok(meta) if !linked(&meta) => directories.push((path, meta)),
                Ok(_) => part.skipped += 1,
                Err(e) => note(&mut part, &path, e),
            }
        }
        self.merge(part);
        directories
            .into_iter()
            .map(|(path, meta)| {
                if let Some((source, kind)) = classify(&path, &names) {
                    Job::Measure(Candidate {
                        search: format!("{} {source} {kind}", path.display()).to_lowercase(),
                        path,
                        source,
                        kind,
                        bytes: 0,
                        files: 0,
                        modified: meta.modified().ok(),
                        created: meta.created().ok(),
                        issue: None,
                        manifest: Arc::new(HashMap::new()),
                    })
                } else {
                    Job::Discover(path)
                }
            })
            .collect()
    }
    fn merge(&self, part: Scan) {
        if part.skipped == 0 && part.warnings.is_empty() {
            return;
        }
        let mut result = self.result.lock().unwrap();
        result.skipped += part.skipped;
        let remaining = 100usize.saturating_sub(result.warnings.len());
        result
            .warnings
            .extend(part.warnings.into_iter().take(remaining));
    }
    fn measure(&self, mut candidate: Candidate) {
        if self.cancel.load(Ordering::Relaxed) {
            return;
        }
        match inventory(
            &candidate.path,
            &self.mounts,
            self.cancel,
            &mut |path, directories, files| {
                self.directories.fetch_add(directories, Ordering::Relaxed);
                self.files.fetch_add(files, Ordering::Relaxed);
                self.progress(path, false);
            },
        ) {
            Ok(manifest) => {
                for entry in manifest.values() {
                    candidate.bytes = candidate.bytes.saturating_add(entry.len);
                    candidate.files += usize::from(!entry.directory);
                    candidate.modified = candidate.modified.max(entry.modified);
                }
                candidate.manifest = Arc::new(manifest);
            }
            Err(e) => candidate.issue = Some(e.to_string()),
        }
        // Interrupted candidates are not presented as fully measured.
        if self.cancel.load(Ordering::Relaxed) {
            return;
        }
        self.bytes.fetch_add(candidate.bytes, Ordering::Relaxed);
        self.found.fetch_add(1, Ordering::Relaxed);
        self.result.lock().unwrap().entries.push(candidate.clone());
        (self.reporter.lock().unwrap().1)(ScanUpdate::Found(candidate));
    }
}

/// Directory-at-a-time DFS bounds open handles and avoids per-file stat calls on
/// Windows by consuming cached DirEntry metadata. Relative hash keys avoid
/// repeating the root path and O(log n) tree insertion for every measured file.
fn inventory(
    root: &Path,
    mounts: &HashSet<PathBuf>,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(&Path, usize, usize),
) -> io::Result<HashMap<PathBuf, Stamp>> {
    let mut manifest = HashMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("Scan cancelled"));
        }
        let dir = root.join(&relative);
        // Fresh directory checks retain link protection when queued paths change.
        let meta = fs::symlink_metadata(&dir)?;
        if mounts.contains(&dir) || linked(&meta) || !meta.is_dir() {
            return Err(io::Error::other(
                "Contains a mount, link, reparse point, or special file; deletion disabled",
            ));
        }
        manifest.insert(relative.clone(), stamp(&meta));
        progress(&dir, 1, 0);
        let mut files = 0;
        for child in fs::read_dir(&dir)? {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::other("Scan cancelled"));
            }
            let child = child?;
            let meta = child.metadata()?; // Does not follow links; cached by Windows enumeration.
            if linked(&meta) || (!meta.is_file() && !meta.is_dir()) {
                return Err(io::Error::other(
                    "Contains a link, reparse point, or special file; deletion disabled",
                ));
            }
            let relative = relative.join(child.file_name());
            if meta.is_dir() {
                pending.push(relative);
            } else {
                manifest.insert(relative, stamp(&meta));
                files += 1;
                if files == 128 {
                    progress(&dir, 0, files);
                    files = 0;
                }
            }
        }
        progress(&dir, 0, files);
    }
    Ok(manifest)
}

/// Reject stale scans and link substitutions. Deletion is deliberately non-recursive:
/// new files appearing after validation prevent directory removal.
pub fn delete(root: &Path, candidates: &[Candidate]) -> DeleteReport {
    let mut report = DeleteReport::default();
    let mounts = sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter_map(|d| d.mount_point().canonicalize().ok())
        .collect();
    for candidate in candidates {
        match delete_one(root, candidate, &mounts) {
            Ok(bytes) => {
                report.bytes = report.bytes.saturating_add(bytes);
                report.removed.push(candidate.path.clone());
            }
            Err(e) => report.failures.push(format!(
                "{}: {e}. Rescan before retrying; some files may have been removed.",
                candidate.path.display()
            )),
        }
    }
    report
}

fn validate_path(root: &Path, path: &Path) -> io::Result<()> {
    if !root.is_absolute()
        || path == root
        || !path.starts_with(root)
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(io::Error::other("Path is outside the scanned root"));
    }
    // Check every ancestor including the root, without following links.
    for ancestor in path.ancestors() {
        if linked(&fs::symlink_metadata(ancestor)?) {
            return Err(io::Error::other(
                "Path now contains a link or reparse point",
            ));
        }
    }
    Ok(())
}

fn delete_one(root: &Path, candidate: &Candidate, mounts: &HashSet<PathBuf>) -> io::Result<u64> {
    if candidate.issue.is_some() || candidate.manifest.is_empty() {
        return Err(io::Error::other("Incomplete scan"));
    }
    validate_path(root, &candidate.path)?;
    let current = inventory(
        &candidate.path,
        mounts,
        &AtomicBool::new(false),
        &mut |_, _, _| {},
    )?;
    if current != *candidate.manifest {
        return Err(io::Error::other("Contents changed since scanning"));
    }
    let mut directories = Vec::new();
    let mut bytes = 0u64;
    for (relative, expected) in &current {
        let path = candidate.path.join(relative);
        validate_path(root, &path)?;
        if expected.directory {
            directories.push(path);
            continue;
        }
        if stamp(&fs::symlink_metadata(&path)?) != *expected {
            return Err(io::Error::other("File changed during deletion"));
        }
        fs::remove_file(&path)?;
        bytes = bytes.saturating_add(expected.len);
    }
    directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in directories {
        validate_path(root, &dir)?;
        fs::remove_dir(dir)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vijideck-cleanup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fn scan(&self) -> Scan {
            scan(&self.0, &AtomicBool::new(false), |_| {})
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }
    /// Reproducible warm-cache traversal benchmark, excluding fixture creation.
    #[test]
    #[ignore]
    fn benchmark_scan() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target/cleanup-benchmark-data");
        if !root.join("ready").exists() {
            for project in 0..48 {
                let dir = root.join(format!("project-{project}"));
                fs::create_dir_all(dir.join("src")).unwrap();
                fs::write(dir.join("Cargo.toml"), "").unwrap();
                for file in 0..256 {
                    fs::write(dir.join(format!("src/file-{file}.rs")), [0; 128]).unwrap();
                }
                for sub in 0..24 {
                    let dir = dir.join(format!("target/debug/{sub}"));
                    fs::create_dir_all(&dir).unwrap();
                    for file in 0..32 {
                        fs::write(dir.join(format!("artifact-{file}")), [0; 128]).unwrap();
                    }
                }
            }
            fs::write(root.join("ready"), "").unwrap();
        }
        for _ in 0..4 {
            let start = Instant::now();
            let result = scan(&root, &AtomicBool::new(false), |_| {});
            assert_eq!(result.entries.len(), 48);
            assert_eq!(
                result.entries.iter().map(|e| e.bytes).sum::<u64>(),
                48 * 24 * 32 * 128
            );
            println!(
                "scan benchmark: {:.3}s, 48 projects, 49,152 files",
                start.elapsed().as_secs_f64()
            );
        }
    }
    #[test]
    fn recognizes_context_sizes_and_avoids_nested_double_counting() {
        let f = Fixture::new();
        f.file("rust/Cargo.toml", "");
        f.file("rust/target/debug/a", "1234");
        f.file("web/package.json", "{}");
        f.file("web/node_modules/pkg/__pycache__/a.pyc", "123456");
        f.file("source/bin/important.txt", "keep");
        f.file("source/build/important.txt", "keep");
        f.file("dotnet/app.csproj", "");
        f.file("dotnet/obj/a", "12");
        f.file(".git/__pycache__/keep", "keep");
        let result = f.scan();
        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.entries.iter().map(|e| e.bytes).sum::<u64>(), 12);
        assert!(result.entries.iter().all(|e| e.issue.is_none()));
    }
    #[test]
    fn cached_candidates_are_remeasured_and_missing_or_unclassified_paths_rejected() {
        let fixture = Fixture::new();
        let root = fixture.0.canonicalize().unwrap();
        let project = root.join("cached");
        std::fs::create_dir_all(project.join("node_modules")).unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();
        std::fs::write(project.join("node_modules/file"), "hello").unwrap();
        let path = project.join("node_modules");
        let cancel = AtomicBool::new(false);
        let candidate = validate_cached(&root, &path, &cancel).unwrap();
        assert_eq!(candidate.bytes, 5);
        std::fs::write(path.join("file"), "updated").unwrap();
        assert_eq!(validate_cached(&root, &path, &cancel).unwrap().bytes, 7);
        std::fs::remove_file(project.join("package.json")).unwrap();
        assert!(validate_cached(&root, &path, &cancel).is_none());
    }

    #[test]
    fn deletes_only_selected_and_rejects_changed_contents_and_wrong_root() {
        let f = Fixture::new();
        f.file("a/__pycache__/a.pyc", "123");
        f.file("b/__pycache__/b.pyc", "456");
        let result = f.scan();
        let chosen = result
            .entries
            .iter()
            .find(|e| e.path.ends_with("a/__pycache__"))
            .unwrap();
        assert_eq!(
            delete(&result.root.join("elsewhere"), std::slice::from_ref(chosen))
                .failures
                .len(),
            1
        );
        f.file("a/__pycache__/new.pyc", "new");
        assert_eq!(
            delete(&result.root, std::slice::from_ref(chosen))
                .failures
                .len(),
            1
        );
        assert!(f.0.join("a/__pycache__/a.pyc").exists());
        let result = f.scan();
        let chosen = result
            .entries
            .iter()
            .find(|e| e.path.ends_with("a/__pycache__"))
            .unwrap();
        let report = delete(&result.root, std::slice::from_ref(chosen));
        assert!(report.failures.is_empty());
        assert_eq!(report.bytes, 6);
        assert!(!chosen.path.exists());
        assert!(f.0.join("b/__pycache__/b.pyc").exists());
    }
    #[test]
    fn cancellation_and_missing_root_are_reported() {
        let f = Fixture::new();
        assert!(scan(&f.0, &AtomicBool::new(true), |_| {}).cancelled);
        assert_eq!(
            scan(&f.0.join("missing"), &AtomicBool::new(false), |_| {})
                .warnings
                .len(),
            1
        );
    }
    #[test]
    fn dev_folders_are_scanned_and_nested_mounts_are_blocked() {
        let f = Fixture::new();
        f.file("dev/project/__pycache__/a.pyc", "123");
        let result = f.scan();
        assert_eq!(result.entries.len(), 1);
        let candidate = &result.entries[0];
        let mounts = HashSet::from([candidate.path.clone()]);
        assert!(delete_one(&result.root, candidate, &mounts).is_err());
        assert!(candidate.path.join("a.pyc").exists());
    }
    #[test]
    fn parallel_scan_matches_serial_and_streams_each_candidate_once() {
        let f = Fixture::new();
        for n in 0..24 {
            f.file(&format!("p{n}/package.json"), "{}");
            f.file(&format!("p{n}/node_modules/pkg/lib/a"), "1234567");
            f.file(&format!("p{n}/node_modules/pkg/__pycache__/a"), "12");
            f.file(&format!("p{n}/source/keep"), "keep");
        }
        let serial = scan_with_workers(&f.0, &AtomicBool::new(false), 1, |_| {});
        let mut streamed = HashSet::new();
        let parallel = scan_with_workers(&f.0, &AtomicBool::new(false), 4, |update| {
            if let ScanUpdate::Found(candidate) = update {
                assert!(streamed.insert(candidate.path));
            }
        });
        assert_eq!(serial.entries.len(), 24);
        assert_eq!(streamed.len(), 24);
        assert_eq!(serial.visited, parallel.visited);
        assert_eq!(parallel.measured_files, 48);
        for (a, b) in serial.entries.iter().zip(&parallel.entries) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.bytes, 9);
            assert_eq!(a.bytes, b.bytes);
            assert_eq!(a.files, b.files);
            assert_eq!(a.manifest, b.manifest);
            assert!(b.issue.is_none());
        }
    }
    #[test]
    fn cancellation_during_work_returns_only_completed_candidates() {
        let f = Fixture::new();
        for n in 0..24 {
            f.file(&format!("p{n}/__pycache__/a"), "123");
        }
        let cancel = AtomicBool::new(false);
        let result = scan_with_workers(&f.0, &cancel, 1, |update| {
            if matches!(update, ScanUpdate::Found(_)) {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(result.cancelled);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].bytes, 3);
        assert!(result.entries[0].issue.is_none());
    }
    #[cfg(windows)]
    #[test]
    fn rejects_windows_junctions_in_contents_and_replaced_ancestors() {
        fn junction(target: &Path, link: &Path) {
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(target.to_string_lossy().replace('/', "\\"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let f = Fixture::new();
        f.file("outside/keep", "keep");
        f.file("project/__pycache__/a.pyc", "123");
        junction(&f.0.join("outside"), &f.0.join("project/__pycache__/link"));
        let result = f.scan();
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].issue.is_some());
        assert_eq!(delete(&result.root, &result.entries).failures.len(), 1);
        fs::remove_dir(f.0.join("project/__pycache__/link")).unwrap();
        let result = f.scan();
        fs::rename(f.0.join("project"), f.0.join("moved")).unwrap();
        junction(&f.0.join("moved"), &f.0.join("project"));
        assert_eq!(delete(&result.root, &result.entries).failures.len(), 1);
        assert!(f.0.join("moved/__pycache__/a.pyc").exists());
        assert!(f.0.join("outside/keep").exists());
    }
    #[cfg(unix)]
    #[test]
    fn skips_links_and_blocks_linked_candidate_contents() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        f.file("source/keep", "keep");
        f.file("__pycache__/a", "a");
        symlink(f.0.join("source"), f.0.join("__pycache__/link")).unwrap();
        symlink(&f.0, f.0.join("loop")).unwrap();
        let result = f.scan();
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].issue.is_some());
        assert_eq!(delete(&result.root, &result.entries).failures.len(), 1);
        assert!(f.0.join("source/keep").exists());
    }
}
