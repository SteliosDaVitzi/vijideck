use crate::ui::*;
use eframe::egui::{self, RichText};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

struct Spec {
    name: &'static str,
    exe: &'static str,
    arg: &'static str,
}
const TOOLS: &[Spec] = &[
    Spec {
        name: "Python",
        exe: "python",
        arg: "--version",
    },
    Spec {
        name: "Python 3",
        exe: "python3",
        arg: "--version",
    },
    Spec {
        name: "Node.js",
        exe: "node",
        arg: "--version",
    },
    Spec {
        name: ".NET",
        exe: "dotnet",
        arg: "--version",
    },
    Spec {
        name: "Java",
        exe: "java",
        arg: "-version",
    },
    Spec {
        name: "Java compiler",
        exe: "javac",
        arg: "-version",
    },
    Spec {
        name: "Rust",
        exe: "rustc",
        arg: "--version",
    },
    Spec {
        name: "Go",
        exe: "go",
        arg: "version",
    },
    Spec {
        name: "Ruby",
        exe: "ruby",
        arg: "--version",
    },
    Spec {
        name: "PHP",
        exe: "php",
        arg: "--version",
    },
    Spec {
        name: "Deno",
        exe: "deno",
        arg: "--version",
    },
    Spec {
        name: "Bun",
        exe: "bun",
        arg: "--version",
    },
    Spec {
        name: "npm",
        exe: "npm",
        arg: "--version",
    },
    Spec {
        name: "npx",
        exe: "npx",
        arg: "--version",
    },
    Spec {
        name: "pnpm",
        exe: "pnpm",
        arg: "--version",
    },
    Spec {
        name: "Yarn",
        exe: "yarn",
        arg: "--version",
    },
    Spec {
        name: "pip",
        exe: "pip",
        arg: "--version",
    },
    Spec {
        name: "pip3",
        exe: "pip3",
        arg: "--version",
    },
    Spec {
        name: "uv",
        exe: "uv",
        arg: "--version",
    },
    Spec {
        name: "Poetry",
        exe: "poetry",
        arg: "--version",
    },
    Spec {
        name: "Cargo",
        exe: "cargo",
        arg: "--version",
    },
    Spec {
        name: "Maven",
        exe: "mvn",
        arg: "--version",
    },
    Spec {
        name: "Gradle",
        exe: "gradle",
        arg: "--version",
    },
    Spec {
        name: "Conda",
        exe: "conda",
        arg: "--version",
    },
    Spec {
        name: "Homebrew",
        exe: "brew",
        arg: "--version",
    },
    Spec {
        name: "winget",
        exe: "winget",
        arg: "--version",
    },
    Spec {
        name: "Chocolatey",
        exe: "choco",
        arg: "--version",
    },
    Spec {
        name: "Scoop",
        exe: "scoop",
        arg: "--version",
    },
    Spec {
        name: "apt",
        exe: "apt",
        arg: "--version",
    },
    Spec {
        name: "dnf",
        exe: "dnf",
        arg: "--version",
    },
    Spec {
        name: "pacman",
        exe: "pacman",
        arg: "--version",
    },
    Spec {
        name: "Docker",
        exe: "docker",
        arg: "--version",
    },
    Spec {
        name: "Podman",
        exe: "podman",
        arg: "--version",
    },
    Spec {
        name: "Git",
        exe: "git",
        arg: "--version",
    },
    Spec {
        name: "CMake",
        exe: "cmake",
        arg: "--version",
    },
    Spec {
        name: "Make",
        exe: "make",
        arg: "--version",
    },
    Spec {
        name: "GCC",
        exe: "gcc",
        arg: "--version",
    },
    Spec {
        name: "Clang",
        exe: "clang",
        arg: "--version",
    },
    Spec {
        name: "Kubernetes CLI",
        exe: "kubectl",
        arg: "version --client",
    },
    Spec {
        name: "Terraform",
        exe: "terraform",
        arg: "--version",
    },
];
struct Entry {
    name: &'static str,
    path: PathBuf,
    on_path: bool,
    version: String,
}
enum ScanEvent {
    Directory(PathBuf),
    Finished(Vec<Entry>),
}

pub struct Environment {
    entries: Vec<Entry>,
    rx: Option<Receiver<ScanEvent>>,
    current_dir: Option<PathBuf>,
    query: String,
    scanned: bool,
    path_confirmation: Option<(PathBuf, bool)>,
    path_notice: Option<String>,
}
impl Environment {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            rx: None,
            current_dir: None,
            query: String::new(),
            scanned: false,
            path_confirmation: None,
            path_notice: None,
        }
    }
    fn scan(&mut self, ctx: &egui::Context) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.current_dir = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut last_update = Instant::now() - Duration::from_millis(100);
            let mut progress = |path: &Path| {
                if last_update.elapsed() >= Duration::from_millis(100) {
                    let _ = tx.send(ScanEvent::Directory(path.to_owned()));
                    ctx.request_repaint();
                    last_update = Instant::now();
                }
            };
            let result = collect(&mut progress);
            let _ = tx.send(ScanEvent::Finished(result));
            ctx.request_repaint();
        });
    }
    pub fn status(&self) -> &'static str {
        if self.rx.is_some() {
            "Scanning developer environment"
        } else {
            "Developer environment"
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.scanned && self.rx.is_none()
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.scanned && self.rx.is_none() {
            self.scanned = true;
            self.scan(ctx);
        }
        let mut finished = false;
        if let Some(rx) = &self.rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ScanEvent::Directory(path) => self.current_dir = Some(path),
                    ScanEvent::Finished(entries) => {
                        self.entries = entries;
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.rx = None;
            self.current_dir = None;
        }
        egui::CentralPanel::default().frame(panel(BG, 28)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Developer environment");
                    ui.label(RichText::new("Installed runtimes, package managers and tools").color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(self.rx.is_none(), egui::Button::new("Refresh")).clicked() { self.scan(ctx); }
                });
            });
            ui.add_space(18.0);
            let focus_search = ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 130.0).clamp(200.0, 520.0);
                egui::Frame::new().fill(PANEL).stroke(egui::Stroke::new(1.0_f32, LINE)).corner_radius(5).inner_margin(egui::Margin::symmetric(12, 9)).show(ui, |ui| {
                    ui.set_width(width - 24.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
                        let center = rect.center() + egui::vec2(-2.0, -2.0);
                        ui.painter().circle_stroke(center, 4.5, egui::Stroke::new(1.2_f32, MUTED));
                        ui.painter().line_segment([center + egui::vec2(3.2, 3.2), center + egui::vec2(7.0, 7.0)], egui::Stroke::new(1.2_f32, MUTED));
                        let input = ui.add_sized([(ui.available_width() - 32.0).max(80.0), 22.0], egui::TextEdit::singleline(&mut self.query).hint_text("Search tools, versions or paths...").frame(false));
                        if focus_search { input.request_focus(); }
                        if !self.query.is_empty() && ui.add(egui::Button::new("x").frame(false)).on_hover_text("Clear search").clicked() { self.query.clear(); }
                    });
                });
                ui.label(RichText::new(format!("{} installations", self.entries.len())).size(12.0).color(MUTED));
            });
            ui.add_space(8.0);
            ui.label(RichText::new("PATH is inherited when VijiDeck starts. Searches are bounded and skip project dependencies and links.").size(11.0).color(MUTED));
            ui.add_space(12.0);
            rule(ui);
            if self.rx.is_some() {
                ui.add_space(12.0);
                ui.horizontal(|ui| { ui.spinner(); ui.label("Scanning installed tools..."); });
                if let Some(path) = &self.current_dir {
                    ui.add(egui::Label::new(RichText::new(path.to_string_lossy()).monospace().size(11.0).color(MUTED)).wrap().selectable(true));
                }
            } else if self.entries.is_empty() {
                ui.add_space(18.0);
                ui.label("No supported tools found. Refresh after changing your environment.");
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let query = self.query.trim().to_lowercase();
                let mut visible = 0;
                for group in ["Python", "Node.js", ".NET", "Java", "Rust", "Go", "Ruby", "PHP", "Deno", "Bun", "Containers", "Git", "Build tools", "System package managers", "Other tools"] {
                    let group_matches = group.to_lowercase().contains(&query);
                    let items: Vec<_> = self.entries.iter().filter(|e| group_for(e.name) == group && (group_matches || format!("{} {} {}", e.name, e.version, e.path.display()).to_lowercase().contains(&query))).collect();
                    if items.is_empty() { continue; }
                    visible += 1;
                    ui.add_space(8.0);
                    egui::Frame::new().fill(SURFACE).corner_radius(4).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let title = format!("{}   ({} {})", group, items.len(), if items.len() == 1 { "installation" } else { "installations" });
                        let mut header = egui::CollapsingHeader::new(RichText::new(title).size(15.0).strong()).id_salt(group);
                        if !query.is_empty() { header = header.open(Some(true)); }
                        header.show(ui, |ui| {
                            ui.add_space(8.0);
                            for category in ["Languages & runtimes", "Package managers", "Other tools"] {
                                let section: Vec<_> = items.iter().copied().filter(|e| category_for(e.name) == category).collect();
                                if section.is_empty() { continue; }
                                ui.add_space(8.0);
                                ui.label(RichText::new(category).size(12.0).color(CYAN).strong());
                                for aliases in group_aliases(&section) {
                                    let e = aliases[0];
                                    ui.add_space(6.0);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(e.name).strong());
                                        ui.label(RichText::new(&e.version).size(12.0).color(MUTED));
                                    });
                                    for e in aliases {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(if e.on_path { "On PATH" } else { "Not on PATH" }).size(11.0).color(if e.on_path { CYAN } else { AMBER }));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.push_id(&e.path, |ui| {
                                                let (menu_rect, menu_response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
                                                if menu_response.hovered() || menu_response.has_focus() { ui.painter().rect_filled(menu_rect, 4.0, LINE); }
                                                let menu_color = if menu_response.hovered() || menu_response.has_focus() { CYAN } else { MUTED };
                                                for offset in [-5.0, 0.0, 5.0] { ui.painter().circle_filled(menu_rect.center() + egui::vec2(0.0, offset), 1.4, menu_color); }
                                                let menu_response = menu_response.on_hover_text("Installation actions");
                                                egui::Popup::menu(&menu_response).show(|ui| {
                                                        if let Some(dir) = e.path.parent() {
                                                            let add = !e.on_path;
                                                            if ui.button(if add { "Add to PATH" } else { "Remove from PATH" }).clicked() {
                                                                self.path_confirmation = Some((dir.to_owned(), add));
                                                                ui.close();
                                                            }
                                                        }
                                                        if let Some(url) = release_url(e.name)
                                                            && ui.button("Official releases / downloads").clicked() {
                                                                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                                                                ui.close();
                                                            }
                                                    });
                                                let (rect, copy) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
                                                if copy.hovered() || copy.has_focus() { ui.painter().rect_filled(rect, 4.0, LINE); }
                                                let color = if copy.hovered() || copy.has_focus() { CYAN } else { MUTED };
                                                let a = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 6.0), egui::vec2(10.0, 12.0));
                                                let b = a.translate(egui::vec2(3.0, 3.0));
                                                ui.painter().rect_stroke(a, 1.0, egui::Stroke::new(1.0_f32, color), egui::StrokeKind::Inside);
                                                ui.painter().rect_filled(b, 1.0, SURFACE);
                                                ui.painter().rect_stroke(b, 1.0, egui::Stroke::new(1.0_f32, color), egui::StrokeKind::Inside);
                                                if copy.on_hover_text("Copy path").clicked() { ui.ctx().copy_text(e.path.to_string_lossy().into_owned()); }
                                            });
                                            ui.add(egui::Label::new(RichText::new(e.path.to_string_lossy()).monospace().size(11.0).color(MUTED)).selectable(true).truncate()).on_hover_text(e.path.to_string_lossy());
                                        });
                                    });
                                    }
                                    ui.add_space(6.0);
                                    rule(ui);
                                }
                            }
                        });
                    });
                }
                if visible == 0 && !query.is_empty() && self.rx.is_none() {
                    ui.add_space(18.0);
                    ui.label("No tools match your search.");
                }
            });
        });
        if let Some((dir, add)) = self.path_confirmation.clone() {
            egui::Window::new(if add { "Add directory to PATH" } else { "Remove directory from PATH" }).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("This changes PATH for the whole directory, including any other tools it contains:");
                ui.add(egui::Label::new(dir.to_string_lossy()).selectable(true).wrap());
                if cfg!(windows) { ui.label("Changes the current user's Windows PATH. System PATH entries cannot be removed here."); }
                else { ui.label("Changes VijiDeck-managed entries in ~/.profile for login shells. Other shell and graphical environments may use different settings."); }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { self.path_confirmation = None; }
                    if ui.button(if add { "Add to user PATH" } else { "Remove from user PATH" }).clicked() {
                        self.path_notice = Some(match crate::environment_path::change(&dir, add) { Ok(message) => message, Err(error) => format!("PATH change failed: {error}") });
                        self.path_confirmation = None;
                    }
                });
            });
        }
        if let Some(message) = self.path_notice.clone() {
            egui::Window::new("PATH change")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(message);
                    if ui.button("OK").clicked() {
                        self.path_notice = None;
                    }
                });
        }
    }
}
// Group only known aliases with the same reported installation and directory.
// Distinct directories or version output remain separate installations.
fn group_aliases<'a>(entries: &[&'a Entry]) -> Vec<Vec<&'a Entry>> {
    let mut groups: Vec<Vec<&Entry>> = Vec::new();
    for &entry in entries {
        let alias = match entry.name {
            "pip" | "pip3" => Some("pip"),
            _ => None,
        };
        if let Some(key) = alias
            && let Some(group) = groups.iter_mut().find(|group| {
                let first = group[0];
                matches!((key, first.name), ("pip", "pip" | "pip3"))
                    && !first.version.starts_with("Version unavailable")
                    && first.version == entry.version
                    && first.path.parent() == entry.path.parent()
                    && first.name != entry.name
            })
        {
            group.push(entry);
            continue;
        }
        groups.push(vec![entry]);
    }
    groups
}
fn category_for(name: &str) -> &'static str {
    match name {
        "Python" | "Python 3" | "Node.js" | ".NET" | "Java" | "Java compiler" | "Rust" | "Go"
        | "Ruby" | "PHP" | "Deno" | "Bun" => "Languages & runtimes",
        "npm" | "npx" | "pnpm" | "Yarn" | "pip" | "pip3" | "uv" | "Poetry" | "Cargo" | "Maven"
        | "Gradle" | "Conda" | "Homebrew" | "winget" | "Chocolatey" | "Scoop" | "apt" | "dnf"
        | "pacman" => "Package managers",
        _ => "Other tools",
    }
}

fn release_url(name: &str) -> Option<&'static str> {
    match name {
        "Python" | "Python 3" => Some("https://www.python.org/downloads/"),
        "Node.js" => Some("https://nodejs.org/en/download"),
        ".NET" => Some("https://dotnet.microsoft.com/download"),
        "Rust" | "Cargo" => Some("https://www.rust-lang.org/tools/install"),
        "Go" => Some("https://go.dev/dl/"),
        "pip" | "pip3" => Some("https://pypi.org/project/pip/"),
        "npm" => Some("https://www.npmjs.com/package/npm"),
        "Docker" => Some("https://docs.docker.com/get-docker/"),
        "Git" => Some("https://git-scm.com/downloads"),
        _ => None,
    }
}
fn group_for(name: &str) -> &'static str {
    match name {
        "Python" | "Python 3" | "pip" | "pip3" | "uv" | "Poetry" | "Conda" => "Python",
        "Node.js" | "npm" | "npx" | "pnpm" | "Yarn" => "Node.js",
        ".NET" => ".NET",
        "Java" | "Java compiler" | "Maven" | "Gradle" => "Java",
        "Rust" | "Cargo" => "Rust",
        "Go" => "Go",
        "Ruby" => "Ruby",
        "PHP" => "PHP",
        "Deno" => "Deno",
        "Bun" => "Bun",
        "Docker" | "Podman" | "Kubernetes CLI" => "Containers",
        "Git" => "Git",
        "CMake" | "Make" | "GCC" | "Clang" => "Build tools",
        "Homebrew" | "winget" | "Chocolatey" | "Scoop" | "apt" | "dnf" | "pacman" => {
            "System package managers"
        }
        _ => "Other tools",
    }
}
fn candidates(dir: &Path, exe: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| dir.join(format!("{exe}.{ext}")))
            .filter(|p| p.is_file())
            .collect()
    }
    #[cfg(not(windows))]
    {
        let p = dir.join(exe);
        if p.is_file() { vec![p] } else { vec![] }
    }
}
fn collect(progress: &mut impl FnMut(&Path)) -> Vec<Entry> {
    let paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    let mut common = Vec::new();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        let h = PathBuf::from(home);
        for p in [
            ".cargo/bin",
            ".local/bin",
            ".dotnet",
            ".deno/bin",
            ".bun/bin",
            "go/bin",
            "scoop/shims",
        ] {
            common.push(h.join(p));
        }
    }
    if cfg!(windows) {
        for v in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(v) {
                let b = PathBuf::from(base);
                for p in [
                    "nodejs",
                    "dotnet",
                    "Git/cmd",
                    "Programs/Python/Python313",
                    "Programs/Python/Python312",
                    "Programs/Python/Python311",
                    "Microsoft/WinGet/Links",
                ] {
                    common.push(b.join(p));
                }
            }
        }
    } else {
        for p in [
            "/usr/local/bin",
            "/usr/bin",
            "/opt/homebrew/bin",
            "/snap/bin",
            "/usr/local/go/bin",
            "/usr/local/share/dotnet",
        ] {
            common.push(PathBuf::from(p));
        }
    }
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        roots.push(PathBuf::from(home));
    }
    if cfg!(windows) {
        for var in [
            "ProgramFiles",
            "ProgramFiles(x86)",
            "LOCALAPPDATA",
            "ProgramData",
        ] {
            if let Some(value) = std::env::var_os(var) {
                roots.push(PathBuf::from(value));
            }
        }
    } else {
        for root in ["/opt", "/usr/local", "/Library/Frameworks", "/Applications"] {
            roots.push(PathBuf::from(root));
        }
    }

    // Traverse each tree once and match filenames during enumeration, rather than
    // probing every tool name in every directory.
    let discovered = discover_candidates(&roots, progress);
    let mut result = Vec::new();
    for (index, spec) in TOOLS.iter().enumerate() {
        let mut seen = std::collections::HashSet::new();
        for (on_path, dirs) in [(true, &paths), (false, &common)] {
            for dir in dirs {
                progress(dir);
                for path in candidates(dir, spec.exe) {
                    let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    if seen.insert(key) {
                        result.push(Entry {
                            name: spec.name,
                            version: version(&path, spec.arg),
                            path,
                            on_path,
                        });
                    }
                }
            }
        }
        for path in &discovered[index] {
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            if seen.insert(key) {
                progress(path.parent().unwrap_or(path));
                result.push(Entry {
                    name: spec.name,
                    version: version(path, spec.arg),
                    path: path.clone(),
                    on_path: false,
                });
            }
        }
    }
    result
}

// Enumerate each directory once and retain only matching executable names.
fn discover_candidates(roots: &[PathBuf], progress: &mut impl FnMut(&Path)) -> Vec<Vec<PathBuf>> {
    use std::collections::{HashMap, HashSet, VecDeque};
    let mut names: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, spec) in TOOLS.iter().enumerate() {
        if cfg!(windows) {
            for ext in ["exe", "cmd", "bat"] {
                names
                    .entry(format!("{}.{}", spec.exe, ext))
                    .or_default()
                    .push(index);
            }
        } else {
            names.entry(spec.exe.to_owned()).or_default().push(index);
        }
    }
    let mut found = vec![Vec::new(); TOOLS.len()];
    for root in roots {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([(root.clone(), 0usize)]);
        let mut visited = 0;
        while let Some((dir, depth)) = queue.pop_front() {
            if visited >= 50_000 {
                break;
            }
            let key = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
            if !seen.insert(key) {
                continue;
            }
            visited += 1;
            progress(&dir);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let filename = entry.file_name();
                let name = filename.to_string_lossy();
                let lookup = if cfg!(windows) {
                    name.to_ascii_lowercase()
                } else {
                    name.to_string()
                };
                if let Some(indices) = names.get(&lookup) {
                    let path = entry.path();
                    if path.is_file() {
                        for &index in indices {
                            found[index].push(path.clone());
                        }
                    }
                }
                if depth >= 7 || !kind.is_dir() || kind.is_symlink() || is_link(&entry.path()) {
                    continue;
                }
                let name = name.to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    ".git"
                        | "node_modules"
                        | "target"
                        | "__pycache__"
                        | ".cache"
                        | "cache"
                        | "caches"
                        | "temp"
                        | "tmp"
                        | "windows"
                        | "$recycle.bin"
                ) {
                    continue;
                }
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    found
}
fn is_link(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return true;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn version(path: &Path, args: &str) -> String {
    // Batch wrappers require a command interpreter. Avoid executing them implicitly.
    if matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("cmd" | "bat")
    ) {
        return "Version unavailable (script wrapper)".into();
    }
    let mut cmd = Command::new(path);
    cmd.args(args.split_whitespace())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let Ok(mut child) = cmd.spawn() else {
        return "Version unavailable".into();
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .ok()
                    .and_then(|out| {
                        let text = if out.stdout.is_empty() {
                            out.stderr
                        } else {
                            out.stdout
                        };
                        String::from_utf8_lossy(&text)
                            .lines()
                            .find(|l| !l.trim().is_empty())
                            .map(|l| l.trim().chars().take(200).collect())
                    })
                    .unwrap_or_else(|| "Version unavailable".into());
            }
            Ok(None) if start.elapsed() < Duration::from_secs(2) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return "Version unavailable (timed out)".into();
            }
        }
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn groups_pip_aliases_only_for_same_verified_installation() {
        let entry = |name, path: &str, version: &str| Entry {
            name,
            path: PathBuf::from(path),
            on_path: true,
            version: version.into(),
        };
        let pip = entry(
            "pip",
            "python/Scripts/pip.exe",
            "pip 25.3 from site-packages",
        );
        let pip3 = entry(
            "pip3",
            "python/Scripts/pip3.exe",
            "pip 25.3 from site-packages",
        );
        let other = entry(
            "pip3",
            "other/Scripts/pip3.exe",
            "pip 25.3 from site-packages",
        );
        let groups = group_aliases(&[&pip, &pip3, &other]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 2);
        let unknown = entry("pip3", "python/Scripts/pip3.exe", "Version unavailable");
        let unknown_pip = entry("pip", "python/Scripts/pip.exe", "Version unavailable");
        assert_eq!(group_aliases(&[&unknown_pip, &unknown]).len(), 2);
    }
    #[test]
    fn discovers_nested_installations_and_skips_project_dependencies() {
        let root = std::env::temp_dir().join(format!("vijideck-env-test-{}", std::process::id()));
        let nested = root.join("versions").join("python").join("bin");
        let skipped = root.join("project").join("node_modules").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&skipped).unwrap();
        let filename = if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        };
        std::fs::write(nested.join(filename), "test").unwrap();
        std::fs::write(skipped.join(filename), "test").unwrap();
        let found = discover_candidates(std::slice::from_ref(&root), &mut |_| {});
        let python = TOOLS.iter().position(|s| s.exe == "python").unwrap();
        assert!(found[python].contains(&nested.join(filename)));
        assert!(!found[python].contains(&skipped.join(filename)));
        std::fs::remove_dir_all(root).unwrap();
    }
}
