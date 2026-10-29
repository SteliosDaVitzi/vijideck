use crate::{
    ports::{self, Snapshot, SocketKey, StopRequest},
    ui::*,
};
use eframe::egui::{self, RichText, Stroke};
use egui_extras::{Column, TableBuilder};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

enum Command {
    Scan,
    Stop(StopRequest),
}
enum Event {
    Snapshot(Snapshot),
    Stopped(Result<String, String>),
}

pub struct VijiDeck {
    background: crate::background::Background,
    storage: Option<crate::storage::Storage>,
    saved_preferences: crate::storage::Preferences,
    active_tool: usize,
    logs_only: bool,
    processes: crate::processes::Processes,
    overview: crate::overview::Overview,
    cleanup: crate::cleanup_ui::Cleanup,
    logs: crate::logs_ui::Logs,
    environment: crate::environment::Environment,
    tx: Sender<Command>,
    rx: Receiver<Event>,
    snapshot: Snapshot,
    selected: Option<SocketKey>,
    query: String,
    protocol: usize,
    listeners_only: bool,
    auto: bool,
    busy: bool,
    last_scan: Instant,
    scanned: bool,
    notice: Option<(String, bool)>,
    confirmation: Option<StopRequest>,
    descending: bool,
    sort: usize,
    inspector_tab: usize,
    #[cfg(feature = "ui-preview")]
    preview_started: Instant,
    #[cfg(feature = "ui-preview")]
    preview_requested: bool,
}

impl VijiDeck {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme(&cc.egui_ctx);
        let (snapshot, selected) = initial_state();
        let scanned = !snapshot.ports.is_empty();
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let repaint = cc.egui_ctx.clone();
        std::thread::Builder::new()
            .name("port-worker".into())
            .spawn(move || {
                let mut system = sysinfo::System::new();
                while let Ok(command) = commands.recv() {
                    let event = match command {
                        Command::Scan => Event::Snapshot(ports::scan(&mut system)),
                        Command::Stop(request) => Event::Stopped(ports::stop(&request)),
                    };
                    if events.send(event).is_err() {
                        break;
                    }
                    repaint.request_repaint();
                }
            })
            .expect("start port worker");
        tx.send(Command::Scan).ok();
        let mut storage = if cfg!(feature = "ui-preview")
            && std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some()
        {
            None
        } else {
            match crate::storage::Storage::open() {
                Ok(store) => Some(store),
                Err(error) => {
                    eprintln!("Could not open preferences: {error}");
                    None
                }
            }
        };
        let preferences = match storage.as_ref().map(|store| store.load_preferences()) {
            Some(Ok(prefs)) => prefs,
            Some(Err(error)) => {
                eprintln!("Could not load preferences: {error}");
                storage = None;
                Default::default()
            }
            None => Default::default(),
        };
        let mut app = Self {
            storage,
            saved_preferences: preferences,
            background: crate::background::Background::new(cc),
            active_tool: 2,
            logs_only: false,
            processes: crate::processes::Processes::new(cc.egui_ctx.clone()),
            overview: crate::overview::Overview::new(cc.egui_ctx.clone()),
            cleanup: crate::cleanup_ui::Cleanup::new(),
            logs: crate::logs_ui::Logs::new(),
            environment: crate::environment::Environment::new(),
            tx,
            rx,
            snapshot,
            selected,
            query: String::new(),
            protocol: preferences.protocol,
            listeners_only: preferences.listeners_only,
            auto: preferences.auto_refresh,
            busy: true,
            last_scan: Instant::now(),
            scanned,
            notice: None,
            confirmation: None,
            descending: preferences.descending,
            sort: preferences.sort,
            inspector_tab: 0,
            #[cfg(feature = "ui-preview")]
            preview_started: Instant::now(),
            #[cfg(feature = "ui-preview")]
            preview_requested: false,
        };
        app.configure_preview();
        #[cfg(feature = "ui-preview")]
        if std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref() == Ok("widget") {
            app.background
                .apply(crate::background::Action::Widget, &cc.egui_ctx);
        }
        if let Some(request) = crate::logs_access::startup_request() {
            app.active_tool = 3;
            app.logs_only = true;
            app.logs.configure_startup(request);
        }
        if !app.logs_only
            && app.storage.is_some()
            && !(cfg!(feature = "ui-preview")
                && std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some())
        {
            app.cleanup.restore_and_sync(&cc.egui_ctx);
            app.environment.scan(&cc.egui_ctx);
        }
        app
    }

    fn configure_preview(&mut self) {
        #[cfg(feature = "ui-preview")]
        if std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some() {
            match std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref() {
                Ok("overview" | "overview-hardware" | "widget") => self.active_tool = 2,
                Ok("processes") => self.active_tool = 4,
                Ok("environment") => self.active_tool = 5,
                Ok("logs") => self.active_tool = 3,
                Ok("logs-list") => {
                    self.active_tool = 3;
                    self.logs.preview();
                }
                Ok("process") => self.inspector_tab = 1,
                Ok("sockets") => self.inspector_tab = 2,
                Ok("empty") => self.selected = None,
                Ok(
                    "cleanup" | "cleanup-confirm" | "cleanup-disks" | "cleanup-scanning"
                    | "cleanup-filters",
                ) => {
                    self.active_tool = 1;
                    self.cleanup.preview();
                }
                Ok("confirm") => {
                    if let Some(key) = &self.selected
                        && let Some(p) = key.pid.and_then(|id| self.snapshot.processes.get(&id))
                    {
                        self.confirmation = Some(StopRequest {
                            socket: key.clone(),
                            start: p.start,
                            name: p.name.clone(),
                            force: true,
                        });
                    }
                }
                _ => {}
            }
            if !matches!(
                std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref(),
                Ok("overview" | "overview-hardware" | "widget")
            ) && self.active_tool == 2
            {
                self.active_tool = 0;
            }
        }
    }

    fn persist_preferences(&mut self) {
        if self.logs_only {
            return;
        }
        let prefs = crate::storage::Preferences {
            auto_refresh: self.auto,
            listeners_only: self.listeners_only,
            protocol: self.protocol,
            sort: self.sort,
            descending: self.descending,
        };
        if prefs != self.saved_preferences
            && let Some(store) = &mut self.storage
        {
            match store.save_preferences(prefs) {
                Ok(()) => self.saved_preferences = prefs,
                Err(error) => {
                    eprintln!("Could not save preferences: {error}");
                    self.storage = None;
                }
            }
        }
    }

    fn refresh(&mut self) {
        if !self.busy {
            self.busy = self.tx.send(Command::Scan).is_ok();
        }
    }

    #[cfg(feature = "ui-preview")]
    fn capture_preview(&mut self, ctx: &egui::Context) {
        let Some(path) = std::env::var_os("VIJIDECK_SCREENSHOT_TO") else {
            return;
        };
        // Let layout, modal sizing, worker results, and hover state settle.
        let ready = match self.active_tool {
            5 => self.environment.preview_ready(),
            4 => self.processes.preview_ready(),
            2 => self.overview.preview_ready(),
            3 => self.logs.preview_ready(),
            _ => true,
        } || self.preview_started.elapsed() > Duration::from_secs(35);
        if !self.preview_requested
            && ready
            && self.preview_started.elapsed() > Duration::from_millis(700)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.preview_requested = true;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let rgba: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    std::path::Path::new(&path),
                    &rgba,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .expect("save UI preview");
                self.background.quitting = true;
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("masthead")
            .exact_height(66.0)
            .frame(panel(PANEL, 0))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(14.0);
                    reactor(ui, 42.0);
                    ui.add_space(12.0);
                    ui.label(RichText::new("VIJIDECK").size(15.0).strong());
                    ui.label(RichText::new("/").size(20.0).color(LINE));
                    ui.label(
                        RichText::new(if self.active_tool == 0 {
                            "Port manager"
                        } else if self.active_tool == 1 {
                            "Disk cleanup"
                        } else if self.active_tool == 5 {
                            "Developer environment"
                        } else if self.active_tool == 3 || self.active_tool == 4 {
                            if self.active_tool == 4 {
                                "Processes"
                            } else {
                                "System logs"
                            }
                        } else {
                            "Overview"
                        })
                        .size(13.0)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(22.0);
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(122.0, 38.0), egui::Sense::click());
                        let hovered = response.hovered() || response.has_focus();
                        let painter = ui.painter();
                        painter.rect_filled(
                            rect,
                            19,
                            if hovered {
                                egui::Color32::from_rgb(30, 65, 73)
                            } else {
                                egui::Color32::from_rgb(18, 42, 49)
                            },
                        );
                        painter.rect_stroke(
                            rect,
                            19,
                            Stroke::new(
                                1.0_f32,
                                CYAN.gamma_multiply(if hovered { 0.8 } else { 0.35 }),
                            ),
                            egui::StrokeKind::Inside,
                        );
                        let icon_center = rect.left_center() + egui::vec2(20.0, 0.0);
                        painter.circle_filled(icon_center, 12.0, CYAN.gamma_multiply(0.15));
                        let icon_rect =
                            egui::Rect::from_center_size(icon_center, egui::vec2(13.0, 10.0));
                        painter.rect_stroke(
                            icon_rect,
                            2,
                            Stroke::new(1.4_f32, CYAN),
                            egui::StrokeKind::Inside,
                        );
                        painter.line_segment(
                            [
                                icon_center + egui::vec2(-4.0, 1.5),
                                icon_center + egui::vec2(0.0, -1.5),
                            ],
                            Stroke::new(1.2_f32, CYAN),
                        );
                        painter.line_segment(
                            [
                                icon_center + egui::vec2(0.0, -1.5),
                                icon_center + egui::vec2(4.0, 1.0),
                            ],
                            Stroke::new(1.2_f32, CYAN),
                        );
                        painter.text(
                            rect.left_center() + egui::vec2(39.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            "Widget",
                            egui::FontId::proportional(13.0),
                            TEXT,
                        );
                        if response
                            .on_hover_text("Keep monitoring in a floating widget")
                            .clicked()
                        {
                            self.background
                                .apply(crate::background::Action::Widget, ctx);
                        }
                        #[cfg(windows)]
                        if let Some(error) = &self.background.error {
                            ui.label(RichText::new("Tray unavailable").color(AMBER))
                                .on_hover_text(error);
                        }
                        ui.label(
                            RichText::new("LOCAL SYSTEM")
                                .monospace()
                                .size(10.0)
                                .color(MUTED),
                        );
                        dot(ui, CYAN);
                    });
                });
            });
    }

    fn tool_rail(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tool_rail")
            .exact_width(72.0)
            .resizable(false)
            .frame(panel(PANEL, 0))
            .show(ctx, |ui| {
                if self.logs_only {
                    ui.add_space(28.0);
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let _ = logs_icon(ui, true);
                    });
                    return;
                }
                ui.add_space(28.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if crate::overview::rail_icon(ui, self.active_tool == 2).clicked() {
                        self.active_tool = 2;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if port_icon(ui, self.active_tool == 0).clicked() {
                        self.active_tool = 0;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if cleanup_icon(ui, self.active_tool == 1).clicked() {
                        self.active_tool = 1;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if logs_icon(ui, self.active_tool == 3).clicked() {
                        self.active_tool = 3;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if tasks_icon(ui, self.active_tool == 4).clicked() {
                        self.active_tool = 4;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if environment_icon(ui, self.active_tool == 5).clicked() {
                        self.active_tool = 5;
                    }
                });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(18.0);
                    caption(ui, "v0.1");
                    ui.add_space(8.0);
                    ui.label(RichText::new("RUST").monospace().size(9.0).color(MUTED));
                });
            });
    }

    fn status_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(31.0)
            .frame(panel(PANEL, 0))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(18.0);
                    dot(
                        ui,
                        if self.active_tool != 0 || self.snapshot.warnings.is_empty() {
                            CYAN
                        } else {
                            AMBER
                        },
                    );
                    ui.label(
                        RichText::new(if self.active_tool == 5 {
                            self.environment.status()
                        } else if self.active_tool == 1 {
                            self.cleanup.status()
                        } else if self.active_tool == 2 {
                            self.overview.status()
                        } else if self.active_tool == 4 {
                            "Process manager"
                        } else if self.active_tool == 3 {
                            self.logs.status()
                        } else if self.auto {
                            "Live monitoring"
                        } else {
                            "Auto refresh paused"
                        })
                        .size(11.0)
                        .color(MUTED),
                    );
                    ui.add_space(16.0);
                    if self.active_tool == 0 {
                        ui.label(
                            RichText::new(format!("{} sockets", self.snapshot.ports.len()))
                                .size(11.0)
                                .color(MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(if self.active_tool == 5 {
                                "Local environment".to_owned()
                            } else if self.active_tool == 1 {
                                "Local files / no automatic deletion".to_owned()
                            } else if self.active_tool == 2 {
                                "Hardware / local telemetry".to_owned()
                            } else if self.active_tool == 4 {
                                "Local processes / refresh every 2 seconds".to_owned()
                            } else if self.active_tool == 3 {
                                "Local logs / read only".to_owned()
                            } else if self.active_tool == 0 && self.auto {
                                "Local ports / refresh every 2 seconds".to_owned()
                            } else if self.active_tool == 0 && !self.scanned {
                                "Waiting for first scan".to_owned()
                            } else {
                                "Waiting for first scan".into()
                            })
                            .size(11.0)
                            .color(MUTED),
                        );
                        if ui.available_width() > 160.0 {
                            ui.label(
                                RichText::new(std::env::consts::OS)
                                    .monospace()
                                    .size(10.0)
                                    .color(MUTED),
                            );
                        }
                    });
                });
            });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let compact = ui.available_height() < 600.0;
        ui.add_space(if compact { 0.0 } else { 6.0 });
        ui.horizontal(|ui| {
            caption(ui, "INSPECTOR");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.selected.is_some()
                    && ui
                        .add(
                            egui::Button::new(RichText::new("Clear").size(11.0).color(MUTED))
                                .frame(false),
                        )
                        .clicked()
                {
                    self.selected = None;
                }
            });
        });
        ui.add_space(if compact { 6.0 } else { 12.0 });
        let Some(key) = self.selected.clone() else {
            ui.add_space(72.0);
            reactor(ui, 56.0);
            ui.add_space(18.0);
            ui.label(RichText::new("Awaiting selection").size(21.0));
            ui.label(
                RichText::new("Choose a socket to inspect its process, endpoints, and activity.")
                    .size(13.0)
                    .color(MUTED),
            );
            return;
        };
        let Some(port) = self.snapshot.ports.iter().find(|p| p.key == key) else {
            ui.label(RichText::new("Socket closed").size(22.0));
            ui.label(
                RichText::new("This socket is no longer present. Select another entry.")
                    .color(MUTED),
            );
            return;
        };
        port_heading(ui, key.local.port(), key.protocol, &port.state, compact);
        ui.add_space(if compact { 10.0 } else { 16.0 });
        let process = key.pid.and_then(|id| self.snapshot.processes.get(&id));
        ui.add(
            egui::Label::new(
                RichText::new(
                    process
                        .map(|p| p.name.as_str())
                        .unwrap_or("Unknown process"),
                )
                .size(20.0),
            )
            .truncate(),
        )
        .on_hover_text(
            process
                .map(|p| p.name.as_str())
                .unwrap_or("Process details unavailable"),
        );
        ui.horizontal(|ui| {
            dot(ui, if port.state == "Listen" { CYAN } else { MUTED });
            caption(
                ui,
                &key.pid
                    .map(|id| format!("PID {id}"))
                    .unwrap_or_else(|| "Owner unavailable".into()),
            );
            if let Some(p) = process {
                caption(ui, &p.status);
            }
        });
        ui.add_space(if compact { 8.0 } else { 14.0 });
        ui.horizontal(|ui| {
            for (i, name) in ["Overview", "Process", "Sockets"].iter().enumerate() {
                if tab(ui, self.inspector_tab == i, name).clicked() {
                    self.inspector_tab = i;
                }
            }
        });
        ui.add_space(10.0);
        // Keep process actions anchored, while long details scroll independently.
        let actions_height = if process.is_some() { 143.0 } else { 0.0 };
        let content_height = (ui.available_height() - actions_height).max(40.0);
        egui::ScrollArea::vertical().id_salt(("inspector", self.inspector_tab)).max_height(content_height)
            .auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(8.0);
                match self.inspector_tab {
                    0 => {
                        if let Some(p) = process {
                            ui.columns(2, |cols| {
                                metric(&mut cols[0], "CPU / per core", &if p.start > 0 { format!("{:.1}%", p.cpu) } else { "Unavailable".into() });
                                metric(&mut cols[1], "Memory / resident", &if p.start > 0 { bytes(p.memory) } else { "Unavailable".into() });
                            });
                            ui.add_space(6.0);
                            caption(ui, "CPU requires two samples; may exceed 100%.");
                            ui.add_space(14.0);
                            rule(ui);
                            ui.add_space(14.0);
                        }
                        field(ui, "Local endpoint", &key.local.to_string());
                        field(ui, "Remote endpoint", &if port.state == "Listen" { "No peer / listening".into() } else { key.remote.map(|r| r.to_string()).unwrap_or_else(|| "Not applicable to UDP".into()) });
                        let exposure = if key.local.ip().is_unspecified() { "All network interfaces" } else if key.local.ip().is_loopback() { "Loopback / this computer" } else { "Specific network interface" };
                        field(ui, "Bind scope", exposure);
                        ui.label(RichText::new("Bind scope does not determine firewall or internet reachability.").size(11.0).color(MUTED));
                        ui.add_space(14.0);
                        if let Some(p) = process {
                            rule(ui); ui.add_space(14.0);
                            field(ui, "User", &p.user);
                            field(ui, "Uptime", &uptime(p));
                        } else {
                            ui.label(RichText::new("Process details may require elevated permissions, or the owner may have exited.").color(MUTED));
                        }
                    }
                    1 => if let Some(p) = process {
                        field(ui, "Executable", &p.executable);
                        field(ui, "Working directory", &p.cwd);
                        field(ui, "User", &p.user);
                        field(ui, "Parent process", &p.parent);
                        let started = (p.start != 0).then(|| chrono::DateTime::from_timestamp(p.start as i64, 0)).flatten()
                            .map(|v| v.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S %:z").to_string()).unwrap_or_else(|| "Unavailable".into());
                        field(ui, "Started", &started);
                        field(ui, "Uptime", &uptime(p));
                        if p.start > 0 {
                            field(ui, "Virtual memory", &bytes(p.virtual_memory));
                            field(ui, "Disk read / written (lifetime)", &format!("{} / {}", bytes(p.read_bytes), bytes(p.written_bytes)));
                        }
                        caption(ui, "Arguments / one per line");
                        panel(BG, 10).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            let arguments = if p.arguments.is_empty() { "Unavailable".into() } else { p.arguments.join("\n") };
                            ui.add(egui::Label::new(RichText::new(arguments).monospace().size(11.0)).wrap().selectable(true));
                        });
                    } else { ui.label("Process details are unavailable."); },
                    _ => {
                        if key.pid.is_some() {
                            let sockets: Vec<_> = self.snapshot.ports.iter().filter(|s| s.key.pid == key.pid).collect();
                            caption(ui, &format!("{} sockets owned by this process", sockets.len()));
                            ui.add_space(8.0);
                            for socket in sockets {
                                let response = ui.add(egui::Button::new(RichText::new(format!("{}  {}", socket.key.protocol, socket.key.local)).monospace().size(11.0))
                                    .selected(socket.key == key).min_size(egui::vec2(ui.available_width(), 34.0)));
                                if response.on_hover_text(&socket.state).clicked() { self.selected = Some(socket.key.clone()); }
                            }
                        } else { ui.label("Socket ownership is unavailable."); }
                    }
                }
            });
        if let Some(p) = process {
            ui.add_space(10.0);
            rule(ui);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                caption(ui, "PROCESS CONTROL");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Copy details").size(11.0).color(CYAN))
                                .frame(false),
                        )
                        .clicked()
                    {
                        ui.ctx().copy_text(format!("{p:#?}\nSocket: {key:#?}"));
                    }
                });
            });
            let can_stop = p.pid > 4 && p.pid != std::process::id() && p.start != 0 && !self.busy;
            ui.horizontal(|ui| {
                let term = sysinfo::SUPPORTED_SIGNALS.contains(&sysinfo::Signal::Term);
                if ui
                    .add_enabled(
                        can_stop && term,
                        egui::Button::new("Terminate").min_size(egui::vec2(100.0, 32.0)),
                    )
                    .on_disabled_hover_text(if !term {
                        "Graceful termination is unavailable on this OS."
                    } else {
                        "Process is protected, unavailable, or a scan is in progress."
                    })
                    .clicked()
                {
                    self.confirmation = Some(StopRequest {
                        socket: key.clone(),
                        start: p.start,
                        name: p.name.clone(),
                        force: false,
                    });
                }
                if ui
                    .add_enabled(
                        can_stop,
                        egui::Button::new(RichText::new("Force kill").color(RED))
                            .fill(RED.gamma_multiply(0.07))
                            .stroke(Stroke::new(1.0_f32, RED.gamma_multiply(0.4)))
                            .min_size(egui::vec2(100.0, 32.0)),
                    )
                    .on_disabled_hover_text(
                        "Process is protected, unavailable, or a scan is in progress.",
                    )
                    .clicked()
                {
                    self.confirmation = Some(StopRequest {
                        socket: key.clone(),
                        start: p.start,
                        name: p.name.clone(),
                        force: true,
                    });
                }
            });
            caption(ui, "Affects the entire process and all its ports.");
        }
    }

    fn main_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading("Port manager");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Refresh"))
                    .on_hover_text("Ctrl / Cmd + R")
                    .clicked()
                {
                    self.refresh();
                }
                if tab(
                    ui,
                    self.auto,
                    if self.auto { "Live / 2s" } else { "Paused" },
                )
                .on_hover_text("Toggle automatic refresh")
                .clicked()
                {
                    self.auto = !self.auto;
                }
            });
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("A clear view of what’s running on your machine.")
                .size(13.0)
                .color(MUTED),
        );
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let count = self
                .snapshot
                .ports
                .iter()
                .filter(|p| p.state == "Listen")
                .count();
            let udp = self
                .snapshot
                .ports
                .iter()
                .filter(|p| p.key.protocol == "UDP")
                .count();
            inline_stat(ui, count, "listening", CYAN);
            ui.add_space(16.0);
            inline_stat(ui, udp, "UDP bound", AMBER);
            ui.add_space(16.0);
            inline_stat(ui, self.snapshot.processes.len(), "processes", TEXT);
        });
        ui.add_space(18.0);
        let focus = ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, LINE))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    search_icon(ui);
                    let width = (ui.available_width() - 35.0).max(100.0);
                    let search = ui.add_sized(
                        [width, 22.0],
                        egui::TextEdit::singleline(&mut self.query)
                            .font(egui::FontId::proportional(13.0))
                            .hint_text("Search ports, processes, PIDs, addresses…")
                            .frame(false),
                    );
                    if focus {
                        search.request_focus();
                    }
                    if !self.query.is_empty()
                        && ui.small_button("x").on_hover_text("Clear search").clicked()
                    {
                        self.query.clear();
                    }
                });
            });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for (i, text) in ["All", "TCP", "UDP"].iter().enumerate() {
                if tab(ui, self.protocol == i, text).clicked() {
                    self.protocol = i;
                }
            }
            ui.add_space(8.0);
            ui.checkbox(&mut self.listeners_only, "Listening / bound only");
        });
        ui.add_space(12.0);
        self.notifications(ui);
        let mut rows: Vec<_> = self
            .snapshot
            .ports
            .iter()
            .filter(|p| {
                (self.protocol == 0 || p.key.protocol == ["", "TCP", "UDP"][self.protocol])
                    && (!self.listeners_only || p.state == "Listen" || p.state == "Bound")
                    && ports::matches(
                        p,
                        p.key.pid.and_then(|id| self.snapshot.processes.get(&id)),
                        &self.query,
                    )
            })
            .collect();
        rows.sort_by(|a, b| {
            let order = match self.sort {
                1 => a.key.protocol.cmp(b.key.protocol),
                2 => a.key.pid.cmp(&b.key.pid),
                3 => {
                    let name = |p: &ports::Port| {
                        p.key
                            .pid
                            .and_then(|id| self.snapshot.processes.get(&id))
                            .map(|p| p.name.as_str())
                            .unwrap_or("")
                    };
                    name(a).cmp(name(b))
                }
                _ => a.key.local.port().cmp(&b.key.local.port()),
            };
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
        ui.horizontal(|ui| {
            caption(ui, &format!("{} socket entries", rows.len()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                caption(ui, "Select a row to inspect");
            });
        });
        ui.add_space(8.0);
        rule(ui);
        if rows.is_empty() {
            ui.add_space(64.0);
            ui.label(
                RichText::new(if self.busy {
                    "Reading local sockets…"
                } else {
                    "No matching sockets"
                })
                .size(22.0),
            );
            ui.label(
                RichText::new(
                    if !self.snapshot.warnings.is_empty() && self.snapshot.ports.is_empty() {
                        "Expand the warning above for details."
                    } else {
                        "Try another search or include all connections."
                    },
                )
                .color(MUTED),
            );
            return;
        }
        let wide = ui.available_width() > 680.0;
        let compact = ui.available_width() < 560.0;
        let mut table = TableBuilder::new(ui)
            .id_salt("ports_table")
            .striped(true)
            .resizable(false)
            .sense(egui::Sense::click())
            .column(Column::exact(if compact { 52.0 } else { 65.0 }))
            .column(Column::exact(if compact { 44.0 } else { 52.0 }))
            .column(Column::exact(if compact { 50.0 } else { 62.0 }))
            .column(Column::remainder().at_least(80.0));
        if wide {
            table = table.column(Column::exact(150.0));
        }
        table
            .column(Column::exact(if compact { 78.0 } else { 91.0 }))
            .header(34.0, |mut header| {
                for (i, name) in ["PORT", "TYPE", "PID", "PROCESS"].iter().enumerate() {
                    header.col(|ui| {
                        ui.spacing_mut().button_padding = egui::vec2(0.0, 3.0);
                        let text = format!(
                            "{name}{}",
                            if self.sort == i {
                                if self.descending { " -" } else { " +" }
                            } else {
                                ""
                            }
                        );
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(text).size(10.0).color(if self.sort == i {
                                        CYAN
                                    } else {
                                        MUTED
                                    }),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Sort; click again to reverse")
                            .clicked()
                        {
                            if self.sort == i {
                                self.descending = !self.descending;
                            } else {
                                self.sort = i;
                                self.descending = false;
                            }
                        }
                    });
                }
                if wide {
                    header.col(|ui| {
                        caption(ui, "BIND ADDRESS");
                    });
                }
                header.col(|ui| {
                    caption(ui, "STATE");
                });
            })
            .body(|body| {
                body.rows(42.0, rows.len(), |mut row| {
                    let port = rows[row.index()];
                    let selected = self.selected.as_ref() == Some(&port.key);
                    row.set_selected(selected);
                    let process = port.key.pid.and_then(|id| self.snapshot.processes.get(&id));
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let rect = ui.max_rect();
                            if selected {
                                ui.painter().vline(
                                    rect.left(),
                                    rect.y_range(),
                                    Stroke::new(2.0_f32, CYAN),
                                );
                            }
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(port.key.local.port().to_string())
                                    .monospace()
                                    .size(14.0)
                                    .color(if selected { CYAN } else { TEXT }),
                            );
                        });
                    });
                    row.col(|ui| {
                        let color = if port.key.protocol == "TCP" {
                            CYAN
                        } else {
                            AMBER
                        };
                        let text = ui.painter().layout_no_wrap(
                            port.key.protocol.to_owned(),
                            egui::FontId::monospace(10.0),
                            color,
                        );
                        // Size the pill from its text, independently of the table's
                        // full-height cell layout, then center it in both directions.
                        let rect = egui::Rect::from_center_size(
                            ui.max_rect().center(),
                            text.size() + egui::vec2(12.0, 6.0),
                        );
                        ui.painter().rect(
                            rect,
                            3,
                            color.gamma_multiply(0.08),
                            Stroke::new(1.0_f32, color.gamma_multiply(0.25)),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter()
                            .galley(rect.center() - text.size() * 0.5, text, color);
                    });
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(
                                    port.key
                                        .pid
                                        .map(|p| p.to_string())
                                        .unwrap_or_else(|| "—".into()),
                                )
                                .monospace()
                                .size(11.0)
                                .color(MUTED),
                            );
                        });
                    });
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        process
                                            .map(|p| p.name.as_str())
                                            .unwrap_or("Unknown / restricted"),
                                    )
                                    .size(13.0),
                                )
                                .truncate(),
                            );
                        });
                    });
                    if wide {
                        row.col(|ui| {
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(port.key.local.ip().to_string())
                                            .monospace()
                                            .size(11.0)
                                            .color(MUTED),
                                    );
                                },
                            );
                        });
                    }
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                dot(ui, if port.state == "Listen" { CYAN } else { MUTED });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&port.state).size(11.0).color(MUTED),
                                    )
                                    .truncate(),
                                );
                            });
                        });
                    });
                    let response = row.response().on_hover_text(format!(
                        "{}\n{}",
                        port.key.local,
                        process
                            .map(|p| p.executable.as_str())
                            .unwrap_or("Owner unavailable")
                    ));
                    if response.clicked() {
                        self.selected = Some(port.key.clone());
                    }
                });
            });
    }

    fn notifications(&mut self, ui: &mut egui::Ui) {
        if let Some((text, error)) = &self.notice {
            let mut dismiss = false;
            egui::Frame::new()
                .fill(SURFACE)
                .inner_margin(10)
                .corner_radius(4)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(if *error { RED } else { CYAN }, text);
                        dismiss = ui.small_button("Dismiss").clicked();
                    });
                });
            ui.add_space(8.0);
            if dismiss {
                self.notice = None;
            }
        }
        if !self.snapshot.warnings.is_empty() {
            egui::CollapsingHeader::new(
                RichText::new("Some socket information is restricted").color(AMBER),
            )
            .show(ui, |ui| {
                for warning in &self.snapshot.warnings {
                    ui.label(warning);
                }
                ui.label(
                    "Try running with administrator/root permissions for restricted information.",
                );
            });
            ui.add_space(8.0);
        }
    }

    fn confirm_stop(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.confirmation.clone() {
            let modal_id = egui::Id::new("confirm_stop");
            let modal = egui::Modal::new(modal_id)
                .area(egui::Modal::default_area(modal_id).fade_in(!cfg!(feature = "ui-preview")))
                .frame(panel(PANEL, 24).stroke(Stroke::new(1.0_f32, LINE)).corner_radius(6))
                .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.add_space(8.0);
                caption(ui, "PROCESS CONTROL");
                ui.add_space(12.0);
                ui.heading(if request.force { "Force kill process?" } else { "Terminate process?" });
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} / PID {}", request.name, request.socket.pid.unwrap_or_default())).color(CYAN));
                ui.add_space(12.0);
                ui.label("This stops the entire process, including its other ports. Unsaved work may be lost.");
                if request.force { ui.label(RichText::new("The process will not have an opportunity to clean up.").color(RED)); }
                ui.add_space(20.0);
                rule(ui);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { self.confirmation = None; }
                    if ui.button(RichText::new(if request.force { "Force kill" } else { "Terminate" }).color(RED)).clicked() {
                        self.busy = self.tx.send(Command::Stop(request)).is_ok();
                        self.confirmation = None;
                    }
                });
                ui.add_space(8.0);
            });
            if modal.should_close() {
                self.confirmation = None;
            }
        }
    }
}

impl eframe::App for VijiDeck {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.persist_preferences();
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.persist_preferences();
        // Screenshot previews must be allowed to exit without becoming a widget.
        #[cfg(feature = "ui-preview")]
        if self.preview_requested && ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        self.background.poll(ctx);
        self.background.sync_window_shape();
        self.cleanup.poll();
        self.overview.poll();
        while let Ok(event) = self.rx.try_recv() {
            self.busy = false;
            match event {
                Event::Snapshot(snapshot) => {
                    self.snapshot = snapshot;
                    self.scanned = true;
                    self.last_scan = Instant::now();
                }
                Event::Stopped(result) => {
                    self.notice = Some(match result {
                        Ok(text) => (text, false),
                        Err(text) => (text, true),
                    });
                    self.refresh();
                }
            }
        }
        if self.active_tool == 0
            && self.auto
            && self.confirmation.is_none()
            && self.last_scan.elapsed() >= Duration::from_secs(2)
        {
            self.refresh();
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        if self.background.widget_visible {
            let can_hide = self.background.can_hide();
            let action = self.overview.widget(ctx, can_hide);
            #[cfg(feature = "ui-preview")]
            self.capture_preview(ctx);
            if let Some(action) = action {
                self.background.apply(action, ctx);
            }
        }
        if self.background.compact {
            return;
        }
        if self.active_tool == 0
            && self.confirmation.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R))
        {
            self.refresh();
        }
        self.top_bar(ctx);
        self.status_bar(ctx);
        self.tool_rail(ctx);
        if self.active_tool == 5 {
            self.environment.show(ctx);
            #[cfg(feature = "ui-preview")]
            self.capture_preview(ctx);
            return;
        }
        if self.active_tool == 4 {
            self.processes.show(ctx);
            #[cfg(feature = "ui-preview")]
            self.capture_preview(ctx);
            return;
        }
        if self.active_tool == 3 {
            self.logs.show(ctx);
            #[cfg(feature = "ui-preview")]
            self.capture_preview(ctx);
            return;
        }
        if self.active_tool == 2 {
            self.overview.show(ctx);
            #[cfg(feature = "ui-preview")]
            self.capture_preview(ctx);
            return;
        }
        if self.active_tool == 1 {
            self.cleanup.show(ctx);
            #[cfg(feature = "ui-preview")]
            self.capture_preview(ctx);
            return;
        }
        egui::SidePanel::right("process_inspector")
            .default_width(350.0)
            .width_range(310.0..=(ctx.content_rect().width() - 548.0).clamp(310.0, 500.0))
            .frame(panel(PANEL, 22))
            .show(ctx, |ui| self.inspector(ui));
        egui::CentralPanel::default()
            .frame(panel(BG, 28))
            .show(ctx, |ui| self.main_panel(ui));
        self.confirm_stop(ctx);
        #[cfg(feature = "ui-preview")]
        self.capture_preview(ctx);
    }
}

fn inline_stat(ui: &mut egui::Ui, value: usize, label: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(value.to_string()).size(20.0).color(color));
        ui.label(RichText::new(label).size(11.0).color(MUTED));
    });
}

fn search_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    ui.painter().circle_stroke(
        rect.center() - egui::vec2(2.0, 2.0),
        4.5,
        Stroke::new(1.2_f32, MUTED),
    );
    ui.painter().line_segment(
        [
            rect.center() + egui::vec2(1.5, 1.5),
            rect.right_bottom() - egui::vec2(2.0, 2.0),
        ],
        Stroke::new(1.2_f32, MUTED),
    );
}

fn bytes(value: u64) -> String {
    if value >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", value as f64 / 1073741824.0)
    } else if value >= 1024 * 1024 {
        format!("{:.1} MiB", value as f64 / 1048576.0)
    } else {
        format!("{:.1} KiB", value as f64 / 1024.0)
    }
}

fn uptime(p: &ports::Process) -> String {
    if p.start == 0 {
        "Unavailable".into()
    } else {
        format!(
            "{}d {}h {}m {}s",
            p.uptime / 86400,
            p.uptime / 3600 % 24,
            p.uptime / 60 % 60,
            p.uptime % 60
        )
    }
}

fn initial_state() -> (Snapshot, Option<SocketKey>) {
    #[cfg(feature = "ui-preview")]
    if std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some() {
        let snapshot = ports::scan(&mut sysinfo::System::new());
        let selected = snapshot
            .ports
            .iter()
            .find(|port| {
                port.state == "Listen"
                    && port.key.pid.is_some_and(|pid| {
                        pid > 4 && snapshot.processes.get(&pid).is_some_and(|p| p.start > 0)
                    })
            })
            .map(|port| port.key.clone());
        return (snapshot, selected);
    }
    (Snapshot::default(), None)
}
