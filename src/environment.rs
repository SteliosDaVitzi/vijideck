use crate::environment_origin::{Origin, OriginDetector};
use crate::ui::*;
use eframe::egui::{self, RichText};
use eframe::emath::GuiRounding;
use std::{
    path::{Path, PathBuf},
    process::Command,
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
    origin: Origin,
    /// The user explicitly allowed running this file outside trusted locations.
    approved: bool,
}
/// Shown for executables found outside trusted install locations that the user
/// has not approved. They are listed but never run automatically.
const UNCHECKED: &str = "Version unavailable (not run: found outside trusted locations)";
/// Result of a user-approved version check: (path, version output).
type Check = (PathBuf, String);
enum ScanEvent {
    Directory(PathBuf),
    Restored(Vec<Entry>),
    Finished(Vec<Entry>),
}

pub struct Environment {
    tab: usize,
    variables: crate::environment_variables::Variables,
    entries: Vec<Entry>,
    rx: Option<Receiver<ScanEvent>>,
    current_dir: Option<PathBuf>,
    query: String,
    scanned: bool,
    path_confirmation: Option<(PathBuf, bool)>,
    path_notice: Option<String>,
    run_confirmation: Option<PathBuf>,
    checks: (mpsc::Sender<Check>, Receiver<Check>),
    icons: std::collections::HashMap<(&'static str, u32), Option<egui::TextureHandle>>,
}
impl Environment {
    pub fn new() -> Self {
        Self {
            tab: if cfg!(feature = "ui-preview")
                && std::env::var_os("VIJIDECK_PREVIEW_VARIABLES").is_some()
            {
                1
            } else {
                0
            },
            variables: Default::default(),
            entries: Vec::new(),
            rx: None,
            current_dir: None,
            query: String::new(),
            scanned: false,
            path_confirmation: None,
            path_notice: None,
            run_confirmation: None,
            checks: mpsc::channel(),
            icons: std::collections::HashMap::new(),
        }
    }
    pub fn scan(&mut self, ctx: &egui::Context) {
        if self.rx.is_some() {
            return;
        }
        self.scanned = true;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.current_dir = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let detector = OriginDetector::new();
            let store = crate::storage::Storage::open_cache();
            let mut approvals = std::collections::HashSet::new();
            if let Some(value) = store
                .as_ref()
                .and_then(|s| s.load_cache("environment").ok().flatten())
            {
                let mut restored = Vec::new();
                let path_dirs = path_dirs();
                let trust = Trust::new();
                if let Some(items) = value.as_array() {
                    for item in items {
                        let Some(spec) = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .and_then(|name| TOOLS.iter().find(|s| s.name == name))
                        else {
                            continue;
                        };
                        let Some(path) = item
                            .get("path")
                            .and_then(|v| serde_json::from_value::<PathBuf>(v.clone()).ok())
                        else {
                            continue;
                        };
                        let Some(parent) = path.parent() else {
                            continue;
                        };
                        if !candidates(parent, spec.exe).contains(&path) {
                            continue;
                        }
                        let on_path = path_dirs.iter().any(|dir| {
                            std::fs::canonicalize(dir).ok() == std::fs::canonicalize(parent).ok()
                                && dir.is_dir()
                        });
                        let approved = item
                            .get("approved")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if approved {
                            approvals.insert(canonical(&path));
                        }
                        let runnable = approved || trust.allows(&path);
                        restored.push(Entry {
                            name: spec.name,
                            version: if runnable {
                                version(&path, spec.arg)
                            } else {
                                UNCHECKED.into()
                            },
                            origin: detector.classify(spec.name, &path),
                            path,
                            on_path,
                            approved,
                        });
                    }
                }
                let _ = tx.send(ScanEvent::Restored(restored));
                ctx.request_repaint();
            }
            let mut last_update = Instant::now() - Duration::from_millis(100);
            let mut progress = |path: &Path| {
                if last_update.elapsed() >= Duration::from_millis(100) {
                    let _ = tx.send(ScanEvent::Directory(path.to_owned()));
                    ctx.request_repaint();
                    last_update = Instant::now();
                }
            };
            let result = collect(&detector, &approvals, &mut progress);
            save_cache(store.as_ref(), &result);
            let _ = tx.send(ScanEvent::Finished(result));
            ctx.request_repaint();
        });
    }
    fn finish_checks(&mut self) {
        let mut changed = false;
        while let Ok((path, version)) = self.checks.1.try_recv() {
            for entry in self.entries.iter_mut().filter(|e| e.path == path) {
                entry.version = version.clone();
                entry.approved = true;
                changed = true;
            }
        }
        // While a scan is running, its Finished handler merges and saves approvals.
        if changed && self.rx.is_none() {
            save_cache(crate::storage::Storage::open_cache().as_ref(), &self.entries);
        }
    }
    fn approve(&mut self, path: PathBuf) {
        let Some(spec) = self
            .entries
            .iter()
            .find(|e| e.path == path)
            .and_then(|e| TOOLS.iter().find(|s| s.name == e.name))
        else {
            return;
        };
        let arg = spec.arg;
        let tx = self.checks.0.clone();
        std::thread::spawn(move || {
            let result = version(&path, arg);
            let _ = tx.send((path, result));
        });
    }
    pub fn status(&self) -> &'static str {
        if self.rx.is_some() {
            "Syncing developer environment"
        } else {
            "Developer environment"
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.tab == 1 || (self.scanned && self.rx.is_none())
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.tab == 0 && !self.scanned && self.rx.is_none() {
            self.scanned = true;
            self.scan(ctx);
        }
        let mut finished = false;
        if let Some(rx) = &self.rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ScanEvent::Directory(path) => self.current_dir = Some(path),
                    ScanEvent::Restored(entries) => self.entries = entries,
                    ScanEvent::Finished(mut entries) => {
                        // Keep approvals granted while the scan was running.
                        for entry in entries.iter_mut().filter(|e| !e.approved) {
                            if let Some(old) = self
                                .entries
                                .iter()
                                .find(|old| old.approved && old.path == entry.path)
                            {
                                entry.version = old.version.clone();
                                entry.approved = true;
                            }
                        }
                        self.entries = entries;
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.rx = None;
            self.current_dir = None;
            save_cache(crate::storage::Storage::open_cache().as_ref(), &self.entries);
        }
        self.finish_checks();
        egui::CentralPanel::default().frame(panel(BG, 28)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Developer environment");
                    ui.label(RichText::new("Installed tools and operating system settings").color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.tab == 0 && ui.add_enabled(self.rx.is_none(), egui::Button::new("Refresh")).clicked() { self.scan(ctx); }
                });
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, 0, "Runtimes & tools");
                ui.selectable_value(&mut self.tab, 1, "Environment variables");
            });
            ui.add_space(12.0);
            if self.tab == 1 { self.variables.show(ui); return; }
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
            ui.label(RichText::new("PATH is inherited when VijiDeck starts. Searches are bounded and skip project dependencies and links. Files found outside trusted install locations are not run until you approve them.").size(11.0).color(MUTED));
            ui.add_space(12.0);
            rule(ui);
            if self.rx.is_some() {
                ui.add_space(12.0);
                ui.horizontal(|ui| { ui.spinner(); ui.label("Syncing installed tools..."); });
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
                    let versions = group_versions(group, &self.entries);
                    let summary = category_summary(&items);
                    visible += 1;
                    ui.add_space(8.0);
                    egui::Frame::new().fill(SURFACE).corner_radius(4).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let pixels = crate::environment_icons::pixel_size(ui.ctx());
                        let icon = self.icons.entry((group, pixels)).or_insert_with(|| {
                            crate::environment_icons::load(ui.ctx(), group, pixels).map_err(|error| {
                                eprintln!("Could not render developer logo for {group}: {error}");
                            }).ok()
                        });
                        let id = ui.make_persistent_id(("environment-group", group));
                        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
                        if !query.is_empty() { state.set_open(true); }
                        let mut title_clicked = false;
                        let mut header = state.show_header(ui, |ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                            if matches!(group, "Rust" | "Deno") {
                                // Preserve upstream monochrome artwork with a light backing.
                                ui.painter().rect_filled(rect, 5.0, egui::Color32::from_rgb(235, 240, 244));
                            }
                            if let Some(icon) = icon {
                                let scale = ui.ctx().pixels_per_point();
                                let size = icon.size_vec2() / scale;
                                let min = (rect.center() - size / 2.0).round_to_pixels(scale);
                                ui.painter().image(icon.id(), egui::Rect::from_min_size(min, size), egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                            } else {
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(16.0), CYAN);
                            }
                            ui.add_space(5.0);
                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    let response = ui.add(egui::Label::new(RichText::new(group).size(15.0).strong()).sense(egui::Sense::click()));
                                    ui.label(RichText::new(&summary).size(12.0).color(MUTED));
                                    title_clicked = response.clicked();
                                });
                                if !versions.is_empty() {
                                    ui.add_space(2.0);
                                    ui.horizontal_wrapped(|ui| {
                                        for version in &versions {
                                            let color = match version.origin {
                                                "Bundled" => AMBER,
                                                "Unknown origin" => MUTED,
                                                _ => CYAN,
                                            };
                                            egui::Frame::new()
                                                .fill(color.gamma_multiply(0.09))
                                                .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.25)))
                                                .corner_radius(8)
                                                .inner_margin(egui::Margin::symmetric(5, 1))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&version.number).monospace().size(10.0).color(color));
                                                }).response.on_hover_text(version.tooltip());
                                        }
                                    });
                                }
                            });
                        });
                        if title_clicked { header.toggle(); }
                        header.body(|ui| {
                            ui.add_space(8.0);
                            for category in ["Languages & runtimes", "Package managers", "Other tools"] {
                                let section: Vec<_> = items.iter().copied().filter(|e| category_for(e.name) == category).collect();
                                if section.is_empty() { continue; }
                                ui.add_space(6.0);
                                egui::Frame::new().fill(PANEL).corner_radius(5).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                let title = format!("{}   Â·   {}", category, section.len());
                                let mut category_header = egui::CollapsingHeader::new(RichText::new(title).size(12.0).color(CYAN).strong())
                                    .id_salt((group, category)).default_open(true);
                                if !query.is_empty() { category_header = category_header.open(Some(true)); }
                                category_header.show(ui, |ui| {
                                for origin in ["Standalone", "Managed", "Bundled", "Unknown origin"] {
                                let origin_entries: Vec<_> = section.iter().copied().filter(|e| e.origin.label() == origin).collect();
                                if origin_entries.is_empty() { continue; }
                                ui.add_space(8.0);
                                ui.label(RichText::new(origin).size(11.0).strong().color(if origin == "Bundled" { AMBER } else { MUTED }));
                                for aliases in group_aliases(&origin_entries) {
                                    let e = aliases[0];
                                    ui.add_space(6.0);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(e.name).strong());
                                        ui.label(RichText::new(&e.version).size(12.0).color(MUTED));
                                    });
                                    for e in aliases {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(if e.on_path { "On PATH" } else { "Not on PATH" }).size(11.0).color(if e.on_path { CYAN } else { AMBER }));
                                        ui.label(RichText::new(e.origin.label()).size(11.0).color(MUTED)).on_hover_text(e.origin.description());
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
                                                        if e.version == UNCHECKED
                                                            && ui.button("Run to check version...").clicked() {
                                                                self.run_confirmation = Some(e.path.clone());
                                                                ui.close();
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
        if let Some(path) = self.run_confirmation.clone() {
            egui::Window::new("Run unverified file?").collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("This file was found outside trusted install locations, so VijiDeck has not run it. Checking its version runs it:");
                ui.add(egui::Label::new(path.to_string_lossy()).selectable(true).wrap());
                ui.label(RichText::new("Only continue if you know where this file came from. Files inside downloaded projects can run any code.").color(AMBER));
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { self.run_confirmation = None; }
                    if ui.button("Run and remember").clicked() {
                        self.run_confirmation = None;
                        self.approve(path);
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
fn category_summary(entries: &[&Entry]) -> String {
    [
        ("Languages & runtimes", "runtime", "runtimes"),
        ("Package managers", "package manager", "package managers"),
        ("Other tools", "other tool", "other tools"),
    ]
    .into_iter()
    .filter_map(|(category, singular, plural)| {
        let count = entries
            .iter()
            .filter(|entry| category_for(entry.name) == category)
            .count();
        (count > 0).then(|| format!("{count} {}", if count == 1 { singular } else { plural }))
    })
    .collect::<Vec<_>>()
    .join(" Â· ")
}
// A group's primary runtime is distinct from its package managers and other tools.
struct VersionPill {
    number: String,
    origin: &'static str,
    evidence: Vec<String>,
}
impl VersionPill {
    fn tooltip(&self) -> String {
        format!("{}\n{}", self.origin, self.evidence.join("\n"))
    }
}

fn group_versions(group: &str, entries: &[Entry]) -> Vec<VersionPill> {
    let mut versions: Vec<VersionPill> = Vec::new();
    for entry in entries {
        let primary = match group {
            "Python" => matches!(entry.name, "Python" | "Python 3"),
            "Node.js" | ".NET" | "Java" | "Rust" | "Go" | "Ruby" | "PHP" | "Deno" | "Bun"
            | "Git" => entry.name == group,
            _ => false,
        };
        if primary && let Some(version) = version_number(&entry.version) {
            let origin = entry.origin.label();
            let evidence = entry.origin.description();
            if let Some(existing) = versions
                .iter_mut()
                .find(|existing| existing.number == version && existing.origin == origin)
            {
                if !existing.evidence.contains(&evidence) {
                    existing.evidence.push(evidence);
                }
            } else {
                versions.push(VersionPill {
                    number: version.to_owned(),
                    origin,
                    evidence: vec![evidence],
                });
            }
        }
    }
    versions.sort_by(|a, b| (&a.number, a.origin).cmp(&(&b.number, b.origin)));
    versions
}

fn version_number(output: &str) -> Option<&str> {
    output.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | ',' | ';'));
        let word = word
            .strip_prefix("go")
            .or_else(|| word.strip_prefix('v'))
            .unwrap_or(word);
        let mut parts = word.split('.');
        let first = parts.next()?;
        let second = parts.next()?;
        if !first.is_empty()
            && first.bytes().all(|c| c.is_ascii_digit())
            && second.as_bytes().first().is_some_and(u8::is_ascii_digit)
        {
            Some(word)
        } else {
            None
        }
    })
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
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default()
}
fn save_cache(store: Option<&crate::storage::Storage>, entries: &[Entry]) {
    let value = serde_json::json!(
        entries
            .iter()
            .map(|entry| serde_json::json!({"name": entry.name, "path": entry.path, "version": entry.version, "on_path": entry.on_path, "approved": entry.approved}))
            .collect::<Vec<_>>()
    );
    if let Some(store) = store
        && let Err(error) = store.save_cache("environment", &value)
    {
        eprintln!("Could not save environment cache: {error}");
    }
}
/// Locations whose executables may be run automatically to read a version:
/// PATH entries, well-known install directories, and administrator-owned trees.
/// Anything else (for example a project folder under the home directory) needs
/// explicit approval, because it may come from untrusted downloads.
struct Trust {
    dirs: Vec<PathBuf>,
    trees: Vec<PathBuf>,
}
impl Trust {
    fn new() -> Self {
        Self {
            dirs: path_dirs()
                .iter()
                .chain(common_dirs().iter())
                .map(|d| canonical(d))
                .collect(),
            trees: trusted_roots().iter().map(|d| canonical(d)).collect(),
        }
    }
    fn allows(&self, path: &Path) -> bool {
        let path = canonical(path);
        path.parent().is_some_and(|parent| self.dirs.iter().any(|d| d == parent))
            || self.trees.iter().any(|tree| path.starts_with(tree))
            || admin_only(&path)
    }
}
/// On macOS and Linux, the equivalent of Program Files: the file and every
/// directory above it are owned by root and not writable by group or others,
/// so only an administrator could have placed it there. Checks the resolved
/// path, so a symlink cannot borrow a trusted location.
#[cfg(unix)]
fn admin_only(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.is_absolute()
        && path.ancestors().all(|p| {
            std::fs::metadata(p).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o022 == 0)
        })
}
#[cfg(not(unix))]
fn admin_only(_path: &Path) -> bool {
    false
}
/// Deep-search roots that only administrators can write to.
fn trusted_roots() -> Vec<PathBuf> {
    if cfg!(windows) {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect()
    } else {
        Vec::new()
    }
}
/// Deep-search roots that users or other software may be able to write to.
/// On macOS and Linux, files found here still run automatically when
/// `admin_only` shows that only root could have written them.
fn untrusted_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        roots.push(PathBuf::from(home));
    }
    if cfg!(windows) {
        for var in ["LOCALAPPDATA", "ProgramData"] {
            if let Some(value) = std::env::var_os(var) {
                roots.push(PathBuf::from(value));
            }
        }
    } else {
        for root in ["/opt", "/usr/local", "/Library/Frameworks", "/Applications"] {
            roots.push(PathBuf::from(root));
        }
    }
    roots
}
fn collect(
    detector: &OriginDetector,
    approvals: &std::collections::HashSet<PathBuf>,
    progress: &mut impl FnMut(&Path),
) -> Vec<Entry> {
    let paths = path_dirs();
    let common = common_dirs();

    // Traverse each tree once and match filenames during enumeration, rather than
    // probing every tool name in every directory.
    let trusted = discover_candidates(&trusted_roots(), progress);
    let untrusted = discover_candidates(&untrusted_roots(), progress);
    let mut result = Vec::new();
    for (index, spec) in TOOLS.iter().enumerate() {
        let mut seen = std::collections::HashSet::new();
        for (on_path, dirs) in [(true, &paths), (false, &common)] {
            for dir in dirs {
                progress(dir);
                for path in candidates(dir, spec.exe) {
                    if seen.insert(canonical(&path)) {
                        result.push(Entry {
                            name: spec.name,
                            version: version(&path, spec.arg),
                            origin: detector.classify(spec.name, &path),
                            path,
                            on_path,
                            approved: false,
                        });
                    }
                }
            }
        }
        for (runnable, found) in [(true, &trusted), (false, &untrusted)] {
            for path in &found[index] {
                let key = canonical(path);
                let runnable = runnable || admin_only(&key);
                let approved = !runnable && approvals.contains(&key);
                if seen.insert(key) {
                    progress(path.parent().unwrap_or(path));
                    result.push(Entry {
                        name: spec.name,
                        version: if runnable || approved {
                            version(path, spec.arg)
                        } else {
                            UNCHECKED.into()
                        },
                        origin: detector.classify(spec.name, path),
                        path: path.clone(),
                        on_path: false,
                        approved,
                    });
                }
            }
        }
    }
    result
}
fn common_dirs() -> Vec<PathBuf> {
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
    common
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
    cmd.args(args.split_whitespace());
    match crate::command::capture(cmd, crate::command::Limits {
        timeout: Duration::from_secs(2),
        output_bytes: 1024 * 1024,
    }) {
        Ok(out) => {
            let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
            String::from_utf8_lossy(&text).lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.trim().chars().take(200).collect())
                .unwrap_or_else(|| "Version unavailable".into())
        }
        Err(crate::command::Error::Timeout) => "Version unavailable (timed out)".into(),
        Err(_) => "Version unavailable".into(),
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn group_versions_deduplicates_runtime_versions_and_excludes_other_tools() {
        let entry = |name, version: &str| Entry {
            name,
            path: PathBuf::from(name),
            on_path: true,
            version: version.into(),
            origin: Origin::Unknown,
            approved: false,
        };
        let entries = vec![
            entry("Python", "Python 3.12.3"),
            entry("Python 3", "Python 3.12.3"),
            entry("Python", "Python 3.13.1"),
            entry("Python", "Version unavailable (script wrapper)"),
            entry("pip", "pip 25.1 from site-packages (python 3.12)"),
            entry("Node.js", "v22.14.0"),
            entry("npm", "10.9.2"),
            entry("Java", "openjdk version \"21.0.4\" 2024-07-16"),
            entry("Java compiler", "javac 17.0.1"),
            entry("Go", "go version go1.23.3 windows/amd64"),
        ];
        let numbers = |group| {
            group_versions(group, &entries)
                .into_iter()
                .map(|pill| pill.number)
                .collect::<Vec<_>>()
        };
        assert_eq!(numbers("Python"), ["3.12.3", "3.13.1"]);
        assert_eq!(numbers("Node.js"), ["22.14.0"]);
        assert_eq!(numbers("Java"), ["21.0.4"]);
        assert_eq!(numbers("Go"), ["1.23.3"]);
        assert!(group_versions("Other tools", &entries).is_empty());
        assert_eq!(version_number("Version unavailable (timed out)"), None);
    }
    #[test]
    fn identical_versions_keep_distinct_origin_pills() {
        let entry = |origin| Entry {
            name: "Python",
            path: PathBuf::from("python"),
            on_path: false,
            version: "Python 3.12.3".into(),
            origin,
            approved: false,
        };
        let entries = vec![
            entry(Origin::Bundled("Blender".into())),
            entry(Origin::Unknown),
            entry(Origin::Bundled("Editor".into())),
        ];
        let pills = group_versions("Python", &entries);
        assert_eq!(pills.len(), 2);
        let bundled = pills.iter().find(|pill| pill.origin == "Bundled").unwrap();
        assert!(bundled.tooltip().contains("Blender"));
        assert!(bundled.tooltip().contains("Editor"));
    }
    #[test]
    fn groups_pip_aliases_only_for_same_verified_installation() {
        let entry = |name, path: &str, version: &str| Entry {
            name,
            path: PathBuf::from(path),
            on_path: true,
            version: version.into(),
            origin: Origin::Unknown,
            approved: false,
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

#[cfg(test)]
mod trust_tests {
    use super::*;
    #[test]
    fn project_folders_are_not_trusted_for_automatic_execution() {
        let root = std::env::temp_dir().join(format!("vijideck-trust-test-{}", std::process::id()));
        let project = root.join("downloads").join("project").join("bin");
        std::fs::create_dir_all(&project).unwrap();
        let file = project.join(if cfg!(windows) { "git.exe" } else { "git" });
        std::fs::write(&file, "test").unwrap();
        assert!(!Trust::new().allows(&file));
        assert!(!admin_only(&canonical(&file)));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn root_owned_system_files_are_admin_only() {
        assert!(admin_only(&canonical(Path::new("/bin/sh"))));
    }
}
