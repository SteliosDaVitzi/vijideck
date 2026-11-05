mod ports_view;
mod tool;

use crate::{
    ports::{self, Snapshot, SocketKey, StopRequest},
    ui::*,
};
use eframe::egui::{self, RichText, Stroke};
use tool::Tool;
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
    active_tool: Tool,
    logs_only: bool,
    processes: crate::processes::Processes,
    overview: crate::overview::Overview,
    cleanup: Option<crate::cleanup_ui::Cleanup>,
    logs: crate::logs_ui::Logs,
    environment: crate::environment::Environment,
    docker: crate::docker::Docker,
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
            active_tool: Tool::Overview,
            logs_only: false,
            processes: crate::processes::Processes::new(cc.egui_ctx.clone()),
            overview: crate::overview::Overview::new(cc.egui_ctx.clone()),
            cleanup: None,
            logs: crate::logs_ui::Logs::new(),
            environment: crate::environment::Environment::new(),
            docker: crate::docker::Docker::new(),
            tx,
            rx,
            snapshot,
            selected,
            query: String::new(),
            protocol: preferences.protocol,
            listeners_only: preferences.listeners_only,
            auto: preferences.auto_refresh,
            busy: false,
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
            app.active_tool = Tool::Logs;
            app.logs_only = true;
            app.logs.configure_startup(request);
        }
        app
    }

    fn configure_preview(&mut self) {
        #[cfg(feature = "ui-preview")]
        if std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some() {
            match std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref() {
                Ok("overview" | "overview-hardware" | "widget") => self.active_tool = Tool::Overview,
                Ok("processes") => self.active_tool = Tool::Processes,
                Ok("environment") => self.active_tool = Tool::Environment,
                Ok("docker") => self.active_tool = Tool::Docker,
                Ok("logs") => self.active_tool = Tool::Logs,
                Ok("logs-list") => {
                    self.active_tool = Tool::Logs;
                    self.logs.preview();
                }
                Ok("process") => self.inspector_tab = 1,
                Ok("sockets") => self.inspector_tab = 2,
                Ok("empty") => self.selected = None,
                Ok(
                    "cleanup" | "cleanup-confirm" | "cleanup-disks" | "cleanup-scanning"
                    | "cleanup-filters",
                ) => {
                    self.active_tool = Tool::Cleanup;
                    self.cleanup.get_or_insert_with(crate::cleanup_ui::Cleanup::new).preview();
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
            ) && self.active_tool == Tool::Overview
            {
                self.active_tool = Tool::Ports;
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
            Tool::Environment => self.environment.preview_ready(),
            Tool::Processes => self.processes.preview_ready(),
            Tool::Overview => self.overview.preview_ready(),
            Tool::Logs => self.logs.preview_ready(),
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
                        RichText::new(self.active_tool.title())
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
                    if crate::overview::rail_icon(ui, self.active_tool == Tool::Overview).clicked() {
                        self.active_tool = Tool::Overview;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if port_icon(ui, self.active_tool == Tool::Ports).clicked() {
                        self.active_tool = Tool::Ports;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if cleanup_icon(ui, self.active_tool == Tool::Cleanup).clicked() {
                        self.active_tool = Tool::Cleanup;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if logs_icon(ui, self.active_tool == Tool::Logs).clicked() {
                        self.active_tool = Tool::Logs;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if tasks_icon(ui, self.active_tool == Tool::Processes).clicked() {
                        self.active_tool = Tool::Processes;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if environment_icon(ui, self.active_tool == Tool::Environment).clicked() {
                        self.active_tool = Tool::Environment;
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if docker_icon(ui, self.active_tool == Tool::Docker).clicked() {
                        self.active_tool = Tool::Docker;
                    }
                });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(18.0);
                    caption(ui, concat!("v", env!("CARGO_PKG_VERSION")));
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
                        if self.active_tool != Tool::Ports || self.snapshot.warnings.is_empty() {
                            CYAN
                        } else {
                            AMBER
                        },
                    );
                    ui.label(
                        RichText::new(if self.active_tool == Tool::Docker {
                            self.docker.status()
                        } else if self.active_tool == Tool::Environment {
                            self.environment.status()
                        } else if self.active_tool == Tool::Cleanup {
                            self.cleanup.as_ref().map_or("Disk cleanup", |cleanup| cleanup.status())
                        } else if self.active_tool == Tool::Overview {
                            self.overview.status()
                        } else if self.active_tool == Tool::Processes {
                            "Process manager"
                        } else if self.active_tool == Tool::Logs {
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
                    if self.active_tool == Tool::Ports {
                        ui.label(
                            RichText::new(format!("{} sockets", self.snapshot.ports.len()))
                                .size(11.0)
                                .color(MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(if self.active_tool == Tool::Docker {
                                "Docker / refresh every 3 seconds".to_owned()
                            } else if self.active_tool == Tool::Environment {
                                "Local environment".to_owned()
                            } else if self.active_tool == Tool::Cleanup {
                                "Local files / no automatic deletion".to_owned()
                            } else if self.active_tool == Tool::Overview {
                                "Hardware / local telemetry".to_owned()
                            } else if self.active_tool == Tool::Processes {
                                "Local processes / refresh every 2 seconds".to_owned()
                            } else if self.active_tool == Tool::Logs {
                                "Local logs / read only".to_owned()
                            } else if self.active_tool == Tool::Ports && self.auto {
                                "Local ports / refresh every 2 seconds".to_owned()
                            } else if self.active_tool == Tool::Ports && !self.scanned {
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
        self.background.sync_window_size(ctx);
        if self.active_tool != Tool::Docker || self.background.compact { self.docker.close_logs(); }
        self.docker.poll();
        if let Some(cleanup) = &mut self.cleanup {
            cleanup.poll();
        }
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
        if self.active_tool == Tool::Ports
            && (!self.scanned || (self.auto && self.last_scan.elapsed() >= Duration::from_secs(2)))
            && self.confirmation.is_none()
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
        if self.active_tool == Tool::Ports
            && self.confirmation.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R))
        {
            self.refresh();
        }
        self.top_bar(ctx);
        self.status_bar(ctx);
        self.tool_rail(ctx);
        match self.active_tool {
            Tool::Docker => self.docker.show(ctx),
            Tool::Environment => self.environment.show(ctx),
            Tool::Processes => self.processes.show(ctx),
            Tool::Logs => self.logs.show(ctx),
            Tool::Overview => self.overview.show(ctx),
            Tool::Cleanup => {
                let cleanup = self.cleanup.get_or_insert_with(|| {
                    let mut cleanup = crate::cleanup_ui::Cleanup::new();
                    cleanup.restore_and_sync(ctx);
                    cleanup
                });
                cleanup.show(ctx);
            },
            Tool::Ports => self.show_ports(ctx),
        }
        #[cfg(feature = "ui-preview")]
        self.capture_preview(ctx);
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
