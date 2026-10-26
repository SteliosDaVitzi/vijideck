use crate::{
    cleanup::{self, DeleteReport, Progress, Scan, ScanUpdate},
    ui::*,
};
use eframe::egui::{self, RichText, Stroke};
use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant, SystemTime},
};

enum Event {
    Update(ScanUpdate),
    Scanned(Scan),
    Deleted(DeleteReport),
}
struct Disk {
    path: PathBuf,
    name: String,
    total: u64,
    free: u64,
    kind: String,
}
fn disks() -> Vec<Disk> {
    let mut disks: Vec<_> = sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .map(|d| Disk {
            path: d.mount_point().to_path_buf(),
            name: d.name().to_string_lossy().into_owned(),
            total: d.total_space(),
            free: d.available_space(),
            kind: match d.kind() {
                sysinfo::DiskKind::SSD => "SSD",
                sysinfo::DiskKind::HDD => "Hard disk",
                _ => "Volume",
            }
            .into(),
        })
        .collect();
    disks.sort_by(|a, b| a.path.cmp(&b.path));
    disks
}

pub struct Cleanup {
    disks: Vec<Disk>,
    choosing: bool,
    selected_disk: Option<PathBuf>,
    filters_open: bool,
    inspected: Option<PathBuf>,
    details_open: bool,
    scan: Option<Scan>,
    selected: HashSet<PathBuf>,
    query: String,
    terms: Vec<String>,
    kind: usize,
    source: String,
    minimum: u64,
    sort: usize,
    descending: bool,
    rows: Vec<usize>,
    dirty: bool,
    busy: bool,
    deleting: bool,
    cancel: Arc<AtomicBool>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    progress: Progress,
    started: Option<Instant>,
    notice: Option<String>,
    failures: Vec<String>,
    confirmation: bool,
    needs_rescan: bool,
}
impl Cleanup {
    pub fn new() -> Self {
        let disks = disks();
        let (tx, rx) = mpsc::channel();
        Self {
            disks,
            choosing: true,
            selected_disk: None,
            filters_open: false,
            inspected: None,
            details_open: false,
            scan: None,
            selected: HashSet::new(),
            query: String::new(),
            terms: Vec::new(),
            kind: 0,
            source: String::new(),
            minimum: 0,
            sort: 0,
            descending: true,
            rows: Vec::new(),
            dirty: true,
            busy: false,
            deleting: false,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            progress: Progress::default(),
            started: None,
            notice: None,
            failures: Vec::new(),
            confirmation: false,
            needs_rescan: false,
        }
    }
    pub fn start_scan(&mut self, ctx: &egui::Context) {
        if !self.can_scan() {
            return;
        }
        let root = self.selected_disk.clone().expect("selected disk");
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.busy = true;
        self.deleting = false;
        self.choosing = false;
        self.started = Some(Instant::now());
        self.scan = Some(Scan {
            root: root.clone(),
            ..Scan::default()
        });
        self.selected.clear();
        self.inspected = None;
        self.rows.clear();
        self.dirty = true;
        self.confirmation = false;
        self.notice = None;
        self.failures.clear();
        self.progress = Progress::default();
        self.needs_rescan = false;
        if let Err(e) = std::thread::Builder::new()
            .name("cleanup-scan".into())
            .spawn(move || {
                let scan = cleanup::scan(&root, &cancel, |update| {
                    tx.send(Event::Update(update)).ok();
                    ctx.request_repaint();
                });
                tx.send(Event::Scanned(scan)).ok();
                ctx.request_repaint();
            })
        {
            self.busy = false;
            self.started = None;
            self.notice = Some(format!("Could not start scan: {e}"));
        }
    }
    fn start_delete(&mut self, ctx: &egui::Context) {
        let Some(scan) = &self.scan else {
            return;
        };
        let candidates = scan
            .entries
            .iter()
            .filter(|e| self.selected.contains(&e.path) && e.issue.is_none())
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return;
        }
        let root = scan.root.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.busy = true;
        self.deleting = true;
        self.confirmation = false;
        if let Err(e) = std::thread::Builder::new()
            .name("cleanup-delete".into())
            .spawn(move || {
                tx.send(Event::Deleted(cleanup::delete(&root, &candidates)))
                    .ok();
                ctx.request_repaint();
            })
        {
            self.busy = false;
            self.deleting = false;
            self.notice = Some(format!("Could not start deletion: {e}"));
        }
    }
    pub fn poll(&mut self) {
        // Bound frame work when many small candidates arrive together.
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(5) {
            let Ok(event) = self.rx.try_recv() else {
                break;
            };
            match event {
                Event::Update(ScanUpdate::Started(root, workers)) => {
                    if let Some(s) = &mut self.scan {
                        s.root = root;
                        s.workers = workers;
                    }
                }
                Event::Update(ScanUpdate::Progress(progress)) => self.progress = progress,
                Event::Update(ScanUpdate::Found(candidate)) => {
                    if let Some(s) = &mut self.scan {
                        s.entries.push(candidate);
                        self.dirty = true;
                    }
                }
                Event::Scanned(scan) => {
                    self.busy = false;
                    self.started = None;
                    self.progress.directories = scan.visited;
                    self.progress.files = scan.measured_files;
                    self.progress.found = scan.entries.len();
                    self.progress.bytes = scan.entries.iter().map(|e| e.bytes).sum();
                    self.scan = Some(scan);
                    self.dirty = true;
                }
                Event::Deleted(report) => {
                    self.busy = false;
                    self.deleting = false;
                    self.needs_rescan = true;
                    self.notice = Some(format!(
                        "Deleted {} folders / {} of file data. Rescan to update remaining results.",
                        report.removed.len(),
                        bytes(report.bytes)
                    ));
                    self.failures = report.failures;
                    if let Some(scan) = &mut self.scan {
                        scan.entries.retain(|e| !report.removed.contains(&e.path));
                    }
                    self.selected.clear();
                    self.dirty = true;
                }
            }
        }
    }
    pub fn status(&self) -> &str {
        if self.deleting {
            "Deleting selected folders"
        } else if self.busy {
            "Scanning / results update live"
        } else {
            "Cleanup / review before deleting"
        }
    }
    fn elapsed(&self) -> Duration {
        self.started
            .map(|t| t.elapsed())
            .unwrap_or_else(|| self.scan.as_ref().map_or(Duration::ZERO, |s| s.elapsed))
    }
    fn can_scan(&self) -> bool {
        !self.busy
            && self
                .selected_disk
                .as_ref()
                .is_some_and(|p| self.disks.iter().any(|d| &d.path == p))
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        egui::CentralPanel::default()
            .frame(panel(BG, 22))
            .show(ctx, |ui| {
                if self.choosing {
                    ui.horizontal(|ui| {
                        ui.heading("Disk cleanup");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.scan.is_some() && ui.button("Back to results").clicked() {
                                self.choosing = false;
                            }
                        });
                    });
                    ui.label(
                        RichText::new(
                            "Choose a disk to find generated files and development caches.",
                        )
                        .color(MUTED),
                    );
                    ui.add_space(20.0);
                    self.disk_picker(ui);
                    return;
                }
                self.scan_summary(ui);
                if let Some(notice) = &self.notice {
                    panel(SURFACE, 10).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(notice);
                    });
                    ui.add_space(6.0);
                }
                self.search_bar(ui);
                self.rebuild_rows();
                self.results(ui);
            });
        if self.details_open {
            self.scan_details(ctx);
        }
        if self.inspected.is_some() {
            self.folder_details(ctx);
        }
        if self.confirmation {
            self.confirm(ctx);
        }
    }
    fn disk_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Choose a disk").size(21.0));
            ui.label(RichText::new(format!("{} volumes", self.disks.len())).color(MUTED));
            if ui.small_button("Refresh disks").clicked() {
                self.disks = disks();
                if !self
                    .selected_disk
                    .as_ref()
                    .is_some_and(|p| self.disks.iter().any(|d| &d.path == p))
                {
                    self.selected_disk = None;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = self
                    .selected_disk
                    .as_ref()
                    .map_or_else(|| "Scan disk".into(), |p| format!("Scan {}", display(p)));
                if ui
                    .add_enabled(
                        self.can_scan(),
                        egui::Button::new(RichText::new(label).color(CYAN))
                            .min_size(egui::vec2(140.0, 36.0)),
                    )
                    .on_disabled_hover_text("Select a disk to scan.")
                    .clicked()
                {
                    self.start_scan(ui.ctx());
                }
            });
        });
        ui.label(
            RichText::new("Select a volume, then start a scan. Nothing is deleted automatically.")
                .color(MUTED),
        );
        ui.add_space(14.0);
        egui::ScrollArea::vertical()
            .id_salt("disk_grid")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                let columns = ((ui.available_width() + 12.0) / 192.0).floor().max(1.0) as usize;
                let size = ((ui.available_width() - (columns - 1) as f32 * 12.0) / columns as f32)
                    .min(210.0);
                if self.disks.is_empty() {
                    ui.label("No mounted disks found. Refresh disks to try again.");
                }
                for chunk in self.disks.chunks(columns) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        for disk in chunk {
                            let selected = self.selected_disk.as_ref() == Some(&disk.path);
                            let response = ui.add(
                                egui::Button::new("")
                                    .min_size(egui::vec2(size, size))
                                    .fill(if selected { SURFACE } else { PANEL })
                                    .stroke(Stroke::new(
                                        if selected { 1.5 } else { 1.0 },
                                        if selected { CYAN } else { LINE },
                                    )),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Button,
                                    true,
                                    selected,
                                    format!("{} {}", display(&disk.path), disk.name),
                                )
                            });
                            let r = response.rect;
                            let p = ui.painter().with_clip_rect(r.shrink(12.0));
                            disk_glyph(
                                &p,
                                egui::Rect::from_min_size(
                                    r.min + egui::vec2(18.0, 16.0),
                                    egui::vec2(32.0, 32.0),
                                ),
                                if selected { CYAN } else { MUTED },
                            );
                            p.text(
                                r.min + egui::vec2(size - 16.0, 22.0),
                                egui::Align2::RIGHT_TOP,
                                &disk.kind,
                                egui::FontId::proportional(11.0),
                                MUTED,
                            );
                            p.text(
                                r.min + egui::vec2(18.0, 58.0),
                                egui::Align2::LEFT_TOP,
                                display(&disk.path),
                                egui::FontId::proportional(26.0),
                                if selected { CYAN } else { TEXT },
                            );
                            p.text(
                                r.min + egui::vec2(18.0, 93.0),
                                egui::Align2::LEFT_TOP,
                                if disk.name.is_empty() {
                                    "Local volume"
                                } else {
                                    &disk.name
                                },
                                egui::FontId::proportional(12.0),
                                MUTED,
                            );
                            let bar = egui::Rect::from_min_size(
                                egui::pos2(r.left() + 18.0, r.bottom() - 58.0),
                                egui::vec2(size - 36.0, 4.0),
                            );
                            p.rect_filled(bar, 2.0, LINE);
                            let used = if disk.total > 0 {
                                1.0 - disk.free as f32 / disk.total as f32
                            } else {
                                0.0
                            };
                            p.rect_filled(
                                egui::Rect::from_min_size(
                                    bar.min,
                                    egui::vec2(bar.width() * used.clamp(0.0, 1.0), 4.0),
                                ),
                                2.0,
                                if used > 0.9 { AMBER } else { CYAN },
                            );
                            p.text(
                                egui::pos2(r.left() + 18.0, r.bottom() - 42.0),
                                egui::Align2::LEFT_TOP,
                                format!("{} free", bytes(disk.free)),
                                egui::FontId::proportional(13.0),
                                TEXT,
                            );
                            p.text(
                                egui::pos2(r.left() + 18.0, r.bottom() - 24.0),
                                egui::Align2::LEFT_TOP,
                                format!("of {}", bytes(disk.total)),
                                egui::FontId::proportional(11.0),
                                MUTED,
                            );
                            if response
                                .on_hover_text(format!(
                                    "{}\n{}\n{} free of {}",
                                    disk.name,
                                    display(&disk.path),
                                    bytes(disk.free),
                                    bytes(disk.total)
                                ))
                                .clicked()
                            {
                                self.selected_disk = Some(disk.path.clone());
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
            });
    }
    fn scan_summary(&mut self, ui: &mut egui::Ui) {
        let compact = ui.ctx().content_rect().height() < 650.0;
        let root = self
            .scan
            .as_ref()
            .map(|s| display(&s.root))
            .unwrap_or_default();
        let cancelled = self.scan.as_ref().is_some_and(|s| s.cancelled);
        let failed = self
            .scan
            .as_ref()
            .is_some_and(|s| s.visited == 0 && !s.warnings.is_empty())
            && !self.busy;
        let cancelling = self.cancel.load(Ordering::Relaxed) && self.busy;
        let (state, color) = if self.deleting {
            ("Deleting", AMBER)
        } else if cancelling {
            ("Stopping", AMBER)
        } else if self.busy {
            ("Scanning", CYAN)
        } else if failed {
            ("Scan failed", RED)
        } else if self.needs_rescan {
            ("Rescan needed", AMBER)
        } else if cancelled {
            ("Partial results", AMBER)
        } else {
            ("Complete", CYAN)
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cleanup results").size(25.0))
                .on_hover_text(&root);
            if compact && let Some(disk) = &self.selected_disk {
                ui.label(RichText::new(display(disk)).size(13.0).color(MUTED));
            }
            ui.add_space(8.0);
            if self.busy {
                ui.add(egui::Spinner::new().size(14.0).color(color));
            } else {
                dot(ui, color);
            }
            ui.label(RichText::new(state).size(12.0).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.busy && !self.deleting {
                    if ui
                        .add_enabled(!cancelling, egui::Button::new("Stop scan"))
                        .clicked()
                    {
                        self.cancel.store(true, Ordering::Relaxed);
                    }
                } else if !self.busy
                    && ui
                        .add_enabled(self.can_scan(), egui::Button::new("Scan again"))
                        .clicked()
                {
                    self.start_scan(ui.ctx());
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Change disk").frame(false))
                    .clicked()
                {
                    self.choosing = true;
                }
            });
        });
        if !compact {
            ui.horizontal(|ui| {
                disk_glyph_ui(ui, 18.0);
                ui.add(egui::Label::new(RichText::new(&root).size(12.0).color(MUTED)).truncate())
                    .on_hover_text(&root);
            });
        }
        ui.add_space(if compact { 5.0 } else { 10.0 });
        let total = self
            .scan
            .as_ref()
            .map_or(0, |s| s.entries.iter().map(|e| e.bytes).sum());
        let matches = self.scan.as_ref().map_or(0, |s| s.entries.len());
        let width = ui.available_width();
        let metric_width = (width - 20.0) / 3.0;
        ui.horizontal(|ui| {
            summary_metric(ui, metric_width, "Detected size", &bytes(total), CYAN);
            summary_metric(ui, metric_width, "Folders found", &number(matches), TEXT);
            summary_metric(
                ui,
                metric_width,
                "Elapsed time",
                &timer(self.elapsed()),
                TEXT,
            );
        });
        ui.horizontal(|ui| {
            let reserve = 125.0;
            ui.allocate_ui_with_layout(
                egui::vec2((ui.available_width() - reserve).max(80.0), 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if self.busy && !self.deleting {
                        let path = display(&self.progress.path);
                        ui.add(
                            egui::Label::new(
                                RichText::new(if path.is_empty() {
                                    "Preparing scan…"
                                } else {
                                    &path
                                })
                                .size(11.0)
                                .color(MUTED),
                            )
                            .truncate(),
                        )
                        .on_hover_text(path);
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "{} folders checked  /  {} files measured",
                                number(self.progress.directories),
                                number(self.progress.files)
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                },
            );
            let issues = self.scan.as_ref().map_or(0, |s| s.warnings.len()) + self.failures.len();
            let details_label = if issues > 0 {
                format!("Scan details  ·  {issues}")
            } else {
                "Scan details".into()
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(details_label).size(11.0).color(CYAN))
                        .min_size(egui::vec2(100.0, 24.0)),
                )
                .on_hover_text("View skipped paths and scan warnings")
                .clicked()
            {
                self.details_open = true;
            }
        });
        ui.add_space(6.0);
        rule(ui);
        ui.add_space(12.0);
    }
    fn reset_filters(&mut self) {
        self.query.clear();
        self.terms.clear();
        self.kind = 0;
        self.source.clear();
        self.minimum = 0;
        self.dirty = true;
    }
    fn search_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let search_width = (ui.available_width() - 264.0).max(180.0);
            ui.allocate_ui_with_layout(
                egui::vec2(search_width, 36.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    panel(PANEL, 6)
                        .stroke(Stroke::new(1.0, LINE))
                        .corner_radius(4)
                        .show(ui, |ui| {
                            ui.set_width(search_width - 12.0);
                            ui.horizontal(|ui| {
                                search_glyph(ui);
                                let input = ui.add_sized(
                                    [(ui.available_width() - 28.0).max(80.0), 24.0],
                                    egui::TextEdit::singleline(&mut self.query)
                                        .hint_text("Search folders, projects, sources…")
                                        .frame(false),
                                );
                                if ui.ctx().input_mut(|i| {
                                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)
                                }) {
                                    input.request_focus();
                                }
                                let clear = !self.query.is_empty()
                                    && ui
                                        .add(egui::Button::new("x").frame(false))
                                        .on_hover_text("Clear search")
                                        .clicked();
                                if clear {
                                    self.query.clear();
                                }
                                if input.changed() || clear {
                                    self.terms = self
                                        .query
                                        .split_whitespace()
                                        .map(str::to_lowercase)
                                        .collect();
                                    self.dirty = true;
                                }
                            });
                        });
                },
            );
            let active = usize::from(!self.source.is_empty()) + usize::from(self.minimum > 0);
            let filter = ui.add(
                egui::Button::new(if active > 0 {
                    format!("Filters ({active})")
                } else {
                    "Filters".into()
                })
                .selected(self.filters_open)
                .min_size(egui::vec2(82.0, 36.0)),
            );
            if filter.clicked() {
                self.filters_open = !self.filters_open;
            }
            egui::Popup::from_response(&filter)
                .open_bool(&mut self.filters_open)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(260.0);
                    ui.label(RichText::new("Filter folders").size(16.0));
                    ui.label(RichText::new("Source").size(12.0).color(MUTED));
                    let sources = self
                        .scan
                        .as_ref()
                        .map(|s| s.entries.iter().map(|e| e.source).collect::<BTreeSet<_>>())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("cleanup_source")
                        .width(260.0)
                        .selected_text(if self.source.is_empty() {
                            "All sources"
                        } else {
                            &self.source
                        })
                        .show_ui(ui, |ui| {
                            self.dirty |= ui
                                .selectable_value(&mut self.source, String::new(), "All sources")
                                .changed();
                            for source in sources {
                                self.dirty |= ui
                                    .selectable_value(&mut self.source, source.into(), source)
                                    .changed();
                            }
                        });
                    ui.label(RichText::new("Minimum size").size(12.0).color(MUTED));
                    egui::ComboBox::from_id_salt("cleanup_minimum")
                        .width(260.0)
                        .selected_text(if self.minimum == 0 {
                            "Any size".into()
                        } else {
                            bytes(self.minimum)
                        })
                        .show_ui(ui, |ui| {
                            for (value, label) in [
                                (0, "Any size"),
                                (1 << 20, "1 MiB"),
                                (100 << 20, "100 MiB"),
                                (1 << 30, "1 GiB"),
                            ] {
                                self.dirty |= ui
                                    .selectable_value(&mut self.minimum, value, label)
                                    .changed();
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui.button("Clear filters").clicked() {
                            self.source.clear();
                            self.minimum = 0;
                            self.dirty = true;
                        }
                        if ui.button("Done").clicked() {
                            ui.close();
                        }
                    });
                });
            egui::ComboBox::from_id_salt("cleanup_sort")
                .width(148.0)
                .height(300.0)
                .selected_text(sort_label(self.sort, self.descending))
                .show_ui(ui, |ui| {
                    for (sort, descending) in [
                        (0, true),
                        (0, false),
                        (1, true),
                        (1, false),
                        (2, true),
                        (2, false),
                        (3, false),
                        (4, false),
                        (5, false),
                    ] {
                        if ui
                            .selectable_label(
                                self.sort == sort && self.descending == descending,
                                sort_label(sort, descending),
                            )
                            .clicked()
                        {
                            self.sort = sort;
                            self.descending = descending;
                            self.dirty = true;
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            let mut counts = [0usize; 3];
            if let Some(scan) = &self.scan {
                for e in &scan.entries {
                    counts[0] += 1;
                    counts[if e.kind == "Build" { 1 } else { 2 }] += 1;
                }
            }
            for (i, name) in ["All", "Build output", "Caches"].iter().enumerate() {
                if tab(
                    ui,
                    self.kind == i,
                    &format!("{name}  {}", number(counts[i])),
                )
                .clicked()
                {
                    self.kind = i;
                    self.dirty = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if (self.kind != 0
                    || !self.query.is_empty()
                    || !self.source.is_empty()
                    || self.minimum > 0)
                    && ui
                        .add(
                            egui::Button::new(RichText::new("Reset").size(12.0).color(CYAN))
                                .frame(false),
                        )
                        .clicked()
                {
                    self.reset_filters();
                }
            });
        });
        if !self.source.is_empty() || self.minimum > 0 {
            ui.horizontal_wrapped(|ui| {
                if !self.source.is_empty()
                    && ui
                        .small_button(format!("{}  x", self.source))
                        .on_hover_text("Remove source filter")
                        .clicked()
                {
                    self.source.clear();
                    self.dirty = true;
                }
                if self.minimum > 0
                    && ui
                        .small_button(format!("At least {}  x", bytes(self.minimum)))
                        .on_hover_text("Remove size filter")
                        .clicked()
                {
                    self.minimum = 0;
                    self.dirty = true;
                }
            });
        }
    }
    fn rebuild_rows(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.rows.clear();
        let Some(scan) = &self.scan else {
            return;
        };
        self.rows.extend(
            scan.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    matches_search(e, &self.terms, self.kind, &self.source, self.minimum)
                })
                .map(|(i, _)| i),
        );
        self.rows.sort_unstable_by(|&a, &b| {
            let a = &scan.entries[a];
            let b = &scan.entries[b];
            let order = match self.sort {
                0 => a.bytes.cmp(&b.bytes),
                1 => a.modified.cmp(&b.modified),
                2 => a.created.cmp(&b.created),
                3 => a.source.cmp(b.source),
                4 => a.kind.cmp(b.kind),
                _ => a.path.cmp(&b.path),
            }
            .then_with(|| a.path.cmp(&b.path));
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
    }
    fn results(&mut self, ui: &mut egui::Ui) {
        let Some(scan) = &self.scan else {
            return;
        };
        let can_select = !self.busy && !self.needs_rescan;
        let selectable = self
            .rows
            .iter()
            .filter(|&&i| scan.entries[i].issue.is_none())
            .count();
        let visible_selected = self
            .rows
            .iter()
            .filter(|&&i| self.selected.contains(&scan.entries[i].path))
            .count();
        let hidden = self.selected.len().saturating_sub(visible_selected);
        let selected_bytes = scan
            .entries
            .iter()
            .filter(|e| self.selected.contains(&e.path))
            .map(|e| e.bytes)
            .sum();
        let footer_height = if hidden > 0 { 83.0 } else { 64.0 };
        let table_height = (ui.available_height() - footer_height - 8.0).max(65.0);
        let wide = ui.available_width() >= 1000.0;
        let table_rect = ui
            .allocate_exact_size(
                egui::vec2(ui.available_width(), table_height),
                egui::Sense::hover(),
            )
            .0;
        let mut table_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(table_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        table_ui.set_clip_rect(table_rect.intersect(ui.clip_rect()));
        if self.rows.is_empty() {
            let filtered = !scan.entries.is_empty();
            let mut reset = false;
            table_ui.scope(|ui| {
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    search_glyph(ui);
                    ui.label(RichText::new(if self.busy { "Searching this disk…" } else if !filtered { "No recognized build output or caches" } else { "No matching folders" }).size(19.0));
                });
                ui.label(RichText::new(if self.busy { "Completed folders appear here as they are measured. You can search while the scan runs." } else if !filtered { "Try another disk. Skipped and unreadable locations are listed in Scan details." } else { "Try a different search or reset your filters." }).color(MUTED));
                if filtered && ui.button("Reset filters").clicked() { reset = true; }
            });
            if reset {
                self.reset_filters();
            }
        } else {
            table_ui.horizontal(|ui| {
                ui.add_space(10.0);
                let mut all = selectable > 0 && visible_selected == selectable;
                if ui
                    .add_enabled(
                        can_select && selectable > 0,
                        egui::Checkbox::without_text(&mut all)
                            .indeterminate(visible_selected > 0 && visible_selected < selectable),
                    )
                    .on_hover_text("Select or clear all visible folders")
                    .changed()
                {
                    for &i in &self.rows {
                        let e = &scan.entries[i];
                        if e.issue.is_none() {
                            if all {
                                self.selected.insert(e.path.clone());
                            } else {
                                self.selected.remove(&e.path);
                            }
                        }
                    }
                }
                let card_width = (table_rect.width() - 16.0).clamp(320.0, 900.0);
                let columns = result_columns(card_width, wide);
                ui.allocate_space(egui::vec2(24.0, 18.0));
                fixed_cell(
                    ui,
                    columns,
                    18.0,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            RichText::new(format!(
                                "Folder  ·  {} matching",
                                number(self.rows.len())
                            ))
                            .size(10.0)
                            .color(MUTED),
                        );
                    },
                );
                fixed_cell(
                    ui,
                    150.0,
                    18.0,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new("Source").size(10.0).color(MUTED));
                    },
                );
                fixed_cell(
                    ui,
                    84.0,
                    18.0,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new("Size").size(10.0).color(MUTED));
                    },
                );
                if wide {
                    fixed_cell(
                        ui,
                        110.0,
                        18.0,
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(RichText::new("Modified").size(10.0).color(MUTED));
                        },
                    );
                }
            });
            table_ui.add_space(5.0);

            egui::ScrollArea::vertical()
                .id_salt(("cleanup_result_cards", wide))
                .auto_shrink([false, false])
                .max_height((table_height - 29.0).max(30.0))
                .show(&mut table_ui, |ui| {
                    ui.set_width(table_rect.width());
                    let card_width = (table_rect.width() - 16.0).max(320.0);
                    for &index in &self.rows {
                        let e = &scan.entries[index];
                        let is_selected = self.selected.contains(&e.path);
                        let mut changed_selection = false;
                        let frame = egui::Frame::new()
                            .fill(if is_selected {
                                egui::Color32::from_rgb(15, 39, 47)
                            } else {
                                PANEL
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if is_selected {
                                    CYAN.gamma_multiply(0.55)
                                } else {
                                    LINE
                                },
                            ))
                            .corner_radius(7)
                            .inner_margin(egui::Margin::symmetric(10, 4));
                        frame.show(ui, |ui| {
                            ui.set_min_width((card_width - 22.0).max(200.0));
                            ui.horizontal(|ui| {
                                let mut selected = is_selected;
                                let checkbox = ui.add_enabled(
                                    can_select && e.issue.is_none(),
                                    egui::Checkbox::without_text(&mut selected),
                                );
                                if checkbox
                                    .on_disabled_hover_text(e.issue.as_deref().unwrap_or(
                                        if self.busy {
                                            "Selection is available after the scan stops."
                                        } else {
                                            "Rescan to update these results."
                                        },
                                    ))
                                    .changed()
                                {
                                    changed_selection = true;
                                }

                                folder_glyph(ui, if e.kind == "Build" { AMBER } else { CYAN });
                                let columns = result_columns(card_width, wide);
                                fixed_cell(
                                    ui,
                                    columns,
                                    38.0,
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        let name = e
                                            .path
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy();
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(name).size(14.0).strong(),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(display(&e.path));
                                        let parent = e.path.parent().unwrap_or(&e.path);
                                        let relative =
                                            parent.strip_prefix(&scan.root).unwrap_or(parent);
                                        let location = if relative.as_os_str().is_empty() {
                                            display(&scan.root)
                                        } else {
                                            display(relative)
                                        };
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(location).size(11.0).color(MUTED),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                                fixed_cell(
                                    ui,
                                    150.0,
                                    38.0,
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        egui::Frame::new()
                                            .fill(egui::Color32::from_rgb(19, 31, 39))
                                            .corner_radius(4)
                                            .inner_margin(egui::Margin::symmetric(7, 4))
                                            .show(ui, |ui| {
                                                ui.vertical(|ui| {
                                                    ui.spacing_mut().item_spacing.y = 1.0;
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(e.source).size(11.0),
                                                        )
                                                        .truncate(),
                                                    );
                                                    ui.label(
                                                        RichText::new(if e.kind == "Build" {
                                                            "Build output"
                                                        } else {
                                                            "Cache"
                                                        })
                                                        .size(10.0)
                                                        .color(MUTED),
                                                    );
                                                });
                                            });
                                    },
                                );
                                fixed_cell(
                                    ui,
                                    84.0,
                                    38.0,
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.label(
                                            RichText::new(if e.issue.is_some() {
                                                "Unavailable".into()
                                            } else {
                                                bytes(e.bytes)
                                            })
                                            .size(14.0)
                                            .strong()
                                            .color(if e.issue.is_some() { AMBER } else { TEXT }),
                                        );
                                        ui.label(
                                            RichText::new(if e.issue.is_some() {
                                                "Needs review".into()
                                            } else {
                                                format!("{} files", number(e.files))
                                            })
                                            .size(10.0)
                                            .color(MUTED),
                                        );
                                    },
                                );
                                if wide {
                                    fixed_cell(
                                        ui,
                                        110.0,
                                        38.0,
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(date(e.modified))
                                                    .size(10.0)
                                                    .color(MUTED),
                                            );
                                        },
                                    );
                                }
                                fixed_cell(
                                    ui,
                                    28.0,
                                    38.0,
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(egui::Button::new("•••").frame(false))
                                            .on_hover_text("Folder details and actions")
                                            .clicked()
                                        {
                                            self.inspected = Some(e.path.clone());
                                        }
                                    },
                                );
                            });
                        });
                        if changed_selection {
                            if is_selected {
                                self.selected.remove(&e.path);
                            } else {
                                self.selected.insert(e.path.clone());
                            }
                        }
                        ui.add_space(5.0);
                    }
                });
        }
        ui.add_space((ui.available_height() - footer_height).max(0.0));
        rule(ui);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(bytes(selected_bytes)).size(22.0).color(
                        if self.selected.is_empty() {
                            MUTED
                        } else {
                            CYAN
                        },
                    ));
                    ui.label(
                        RichText::new(format!("{} selected", number(self.selected.len())))
                            .size(12.0)
                            .color(MUTED),
                    );
                    if !self.selected.is_empty()
                        && ui.add(egui::Button::new("Clear").frame(false)).clicked()
                    {
                        self.selected.clear();
                    }
                });
                ui.label(
                    RichText::new(if self.busy {
                        "Selection becomes available when the scan stops."
                    } else if self.needs_rescan {
                        "Rescan to update remaining folders before deleting again."
                    } else {
                        "Review selected folders before permanently deleting them."
                    })
                    .size(11.0)
                    .color(MUTED),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        can_select && !self.selected.is_empty(),
                        egui::Button::new(
                            RichText::new("Review deletion").color(if self.selected.is_empty() {
                                MUTED
                            } else {
                                RED
                            }),
                        )
                        .min_size(egui::vec2(145.0, 36.0)),
                    )
                    .clicked()
                {
                    self.confirmation = true;
                }
            });
        });
        if hidden > 0 {
            ui.label(RichText::new(format!("{hidden} selected folders are hidden by your filters; they are included in the review.")).size(11.0).color(AMBER));
        }
    }
    fn scan_details(&mut self, ctx: &egui::Context) {
        let Some(scan) = &self.scan else {
            return;
        };
        egui::Window::new("Scan details").open(&mut self.details_open).default_width(540.0).resizable(true).show(ctx, |ui| {
            field(ui, "Scanned disk", &display(&scan.root));
            ui.label(format!("{} directories checked / {} files measured", number(self.progress.directories), number(self.progress.files)));
            ui.label(format!("{} paths skipped", number(scan.skipped)));
            ui.label(RichText::new("Source control metadata, system folders, links, other mounted disks, and unreadable paths are skipped.").color(MUTED));
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for warning in &scan.warnings { ui.colored_label(AMBER, warning); }
                for failure in &self.failures { ui.colored_label(RED, failure); }
                if scan.warnings.is_empty() && self.failures.is_empty() { ui.label("No read or deletion errors reported."); }
            });
        });
    }
    fn folder_details(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.inspected else {
            return;
        };
        let Some(e) = self
            .scan
            .as_ref()
            .and_then(|s| s.entries.iter().find(|e| &e.path == path))
        else {
            self.inspected = None;
            return;
        };
        let mut open = true;
        egui::Window::new("Folder details")
            .id(egui::Id::new("cleanup_folder_details"))
            .open(&mut open)
            .default_width(480.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading(e.path.file_name().unwrap_or_default().to_string_lossy());
                field(ui, "Full path", &display(&e.path));
                ui.horizontal(|ui| {
                    ui.label(e.source);
                    ui.label(RichText::new(e.kind).color(MUTED));
                });
                rule(ui);
                field(
                    ui,
                    "Logical file size",
                    &format!("{} / {} files", bytes(e.bytes), number(e.files)),
                );
                field(ui, "Newest modification", &date(e.modified));
                field(ui, "Folder created", &date(e.created));
                if let Some(issue) = &e.issue {
                    ui.colored_label(AMBER, issue);
                }
                if ui.button("Copy path").clicked() {
                    ui.ctx().copy_text(display(&e.path));
                }
            });
        if !open {
            self.inspected = None;
        }
    }
    fn confirm(&mut self, ctx: &egui::Context) {
        let Some(scan) = &self.scan else {
            return;
        };
        let entries = scan
            .entries
            .iter()
            .filter(|e| self.selected.contains(&e.path))
            .collect::<Vec<_>>();
        let mut delete = false;
        let modal = egui::Modal::new(egui::Id::new("cleanup_confirm")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new(format!("Delete {} folders?", entries.len())).size(25.0));
            ui.label(RichText::new(bytes(entries.iter().map(|e| e.bytes).sum())).size(30.0).color(CYAN));
            ui.label("Deletion is permanent and bypasses the Recycle Bin / Trash."); ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() * 0.35).clamp(120.0, 260.0)).show(ui, |ui| {
                for e in &entries {
                    panel(SURFACE, 10).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| { ui.label(RichText::new(e.path.file_name().unwrap_or_default().to_string_lossy()).strong()); ui.label(RichText::new(bytes(e.bytes)).color(MUTED)); });
                        ui.add(egui::Label::new(RichText::new(display(&e.path)).size(11.0).color(MUTED)).wrap().selectable(true));
                    });
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Close builds and apps using these folders. Dependencies may need downloading again. Changed or locked contents may prevent complete deletion.").size(12.0).color(AMBER));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() { self.confirmation = false; }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("Delete permanently").color(RED)).clicked() { delete = true; }
                });
            });
        });
        if modal.should_close() {
            self.confirmation = false;
        }
        if delete {
            self.start_delete(ctx);
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self) {
        let mode = std::env::var("VIJIDECK_PREVIEW_VIEW").unwrap_or_default();
        if mode == "cleanup-disks" {
            return;
        }
        if let Some(root) = std::env::var_os("VIJIDECK_PREVIEW_CLEANUP_ROOT") {
            let scan = cleanup::scan(&PathBuf::from(root), &AtomicBool::new(false), |_| {});
            self.selected_disk = self
                .disks
                .iter()
                .find(|d| {
                    d.path
                        .canonicalize()
                        .is_ok_and(|p| scan.root.starts_with(p))
                })
                .map(|d| d.path.clone());
            self.progress = Progress {
                directories: scan.visited,
                files: scan.measured_files,
                bytes: scan.entries.iter().map(|e| e.bytes).sum(),
                found: scan.entries.len(),
                path: scan.root.clone(),
            };
            self.choosing = false;
            self.scan = Some(scan);
            self.dirty = true;
            if mode == "cleanup-confirm" {
                self.selected = self
                    .scan
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .filter(|e| e.issue.is_none())
                    .map(|e| e.path.clone())
                    .collect();
                self.confirmation = true;
            } else if mode == "cleanup-scanning" {
                self.busy = true;
                self.started = Some(Instant::now() - Duration::from_secs(72));
            } else if mode == "cleanup-filters" {
                self.filters_open = true;
            }
        }
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn matches_search(
    e: &cleanup::Candidate,
    terms: &[String],
    kind: usize,
    source: &str,
    minimum: u64,
) -> bool {
    (kind == 0 || e.kind == ["", "Build", "Cache"][kind])
        && (source.is_empty() || source == e.source)
        && e.bytes >= minimum
        && terms.iter().all(|q| e.search.contains(q))
}
fn display(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}
fn summary_metric(ui: &mut egui::Ui, width: f32, label: &str, value: &str, color: egui::Color32) {
    let compact = ui.ctx().content_rect().height() < 650.0;
    ui.allocate_ui_with_layout(
        egui::vec2(width, if compact { 45.0 } else { 58.0 }),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(width);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(label).size(11.0).color(MUTED));
            ui.label(
                RichText::new(value)
                    .size(if compact { 24.0 } else { 27.0 })
                    .color(color),
            );
        },
    );
}
fn disk_glyph_ui(ui: &mut egui::Ui, size: f32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    disk_glyph(ui.painter(), r, MUTED);
}
fn folder_glyph(ui: &mut egui::Ui, color: egui::Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
    let points = vec![
        r.min + egui::vec2(2.0, 7.0),
        r.min + egui::vec2(10.0, 7.0),
        r.min + egui::vec2(12.0, 10.0),
        r.min + egui::vec2(22.0, 10.0),
        r.min + egui::vec2(22.0, 21.0),
        r.min + egui::vec2(2.0, 21.0),
        r.min + egui::vec2(2.0, 7.0),
    ];
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.2, color)));
}
fn sort_label(sort: usize, descending: bool) -> &'static str {
    match (sort, descending) {
        (0, true) => "Largest first",
        (0, false) => "Smallest first",
        (1, true) => "Recently modified",
        (1, false) => "Oldest modified",
        (2, true) => "Newest created",
        (2, false) => "Oldest created",
        (3, true) => "Source Z-A",
        (3, false) => "Source A-Z",
        (4, _) => "Type",
        (5, true) => "Path Z-A",
        _ => "Path A-Z",
    }
}
fn result_columns(card_width: f32, wide: bool) -> f32 {
    let fixed = 22.0 + 18.0 + 24.0 + 60.0 + 150.0 + 84.0 + 28.0;
    let fixed = fixed + if wide { 110.0 } else { 0.0 };
    (card_width - fixed).max(170.0)
}
fn fixed_cell(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    layout: egui::Layout,
    add: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(egui::vec2(width, height), layout, |cell| {
        cell.set_min_width(width);
        add(cell);
    });
}
fn number(n: usize) -> String {
    let s = n.to_string();
    let len = s.len();
    s.chars()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && (len - i).is_multiple_of(3) {
                vec![',', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
fn timer(d: Duration) -> String {
    let s = d.as_secs();
    format!(
        "{:02}:{:02}:{:02}.{:02}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        d.subsec_millis() / 10
    )
}
fn search_glyph(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    ui.painter().circle_stroke(
        r.center() - egui::vec2(2.0, 2.0),
        5.0,
        Stroke::new(1.4, MUTED),
    );
    ui.painter().line_segment(
        [
            r.center() + egui::vec2(2.0, 2.0),
            r.right_bottom() - egui::vec2(2.0, 2.0),
        ],
        Stroke::new(1.4, MUTED),
    );
}
fn date(t: Option<SystemTime>) -> String {
    t.map(|t| {
        chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
    .unwrap_or_else(|| "Unavailable".into())
}
fn bytes(v: u64) -> String {
    for (unit, divisor) in [
        ("TiB", 1u64 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
    ] {
        if v >= divisor {
            return format!("{:.1} {unit}", v as f64 / divisor as f64);
        }
    }
    format!("{v} B")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_search_and_selection_survive_final_result_reordering() {
        let target = std::env::current_dir().unwrap().join("target");
        let root = target.join(format!(
            "cleanup-search-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Fixture(PathBuf, PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let resolved = self.0.canonicalize().unwrap();
                assert!(resolved.starts_with(self.1.canonicalize().unwrap()) && resolved != self.1);
                std::fs::remove_dir_all(resolved).unwrap();
            }
        }
        std::fs::create_dir_all(&root).unwrap();
        let _fixture = Fixture(root.clone(), target);
        for name in ["project-alpha", "project-beta"] {
            let project = root.join(name);
            std::fs::create_dir_all(project.join("target/debug")).unwrap();
            std::fs::write(project.join("Cargo.toml"), "").unwrap();
            std::fs::write(project.join("target/debug/file"), "12345678").unwrap();
        }
        let mut ui = Cleanup::new();
        ui.scan = Some(Scan::default());
        let tx = ui.tx.clone();
        let scan = cleanup::scan(&root, &AtomicBool::new(false), |update| {
            tx.send(Event::Update(update)).unwrap();
        });
        ui.poll();
        ui.terms = "PROJECT-ALPHA CARGO"
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        ui.source = "Rust / Cargo".into();
        ui.kind = 1;
        ui.minimum = 8;
        ui.rebuild_rows();
        assert_eq!(ui.rows.len(), 1);
        let chosen = ui.scan.as_ref().unwrap().entries[ui.rows[0]].path.clone();
        ui.selected.insert(chosen.clone());
        ui.tx.send(Event::Scanned(scan)).unwrap();
        ui.poll();
        ui.rebuild_rows();
        assert_eq!(ui.rows.len(), 1);
        assert_eq!(ui.scan.as_ref().unwrap().entries[ui.rows[0]].path, chosen);
        assert!(ui.selected.contains(&chosen));
        ui.minimum = 9;
        ui.dirty = true;
        ui.rebuild_rows();
        assert!(ui.rows.is_empty());
        ui.minimum = 0;
        ui.kind = 2;
        ui.dirty = true;
        ui.rebuild_rows();
        assert!(ui.rows.is_empty());
        assert!(ui.selected.contains(&chosen));
    }
}
