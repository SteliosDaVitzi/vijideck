use crate::{
    services::{self, Action, Request, Scope, Service},
    ui::*,
};
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

struct Event {
    snapshot: Result<Vec<Service>, String>,
    notice: Option<Result<String, String>>,
}

pub struct Services {
    scope: Scope,
    services: Vec<Service>,
    selected: Option<String>,
    query: String,
    state: usize,
    startup: usize,
    auto: bool,
    scanned: bool,
    last_scan: Instant,
    pending: Option<Receiver<Event>>,
    mutating: bool,
    error: Option<String>,
    notice: Option<(String, bool)>,
    confirmation: Option<Request>,
    #[cfg(feature = "ui-preview")]
    preview: bool,
}

impl Services {
    pub fn new() -> Self {
        Self {
            scope: Scope::System,
            services: Vec::new(),
            selected: None,
            query: String::new(),
            state: 0,
            startup: 0,
            auto: true,
            scanned: false,
            last_scan: Instant::now(),
            pending: None,
            mutating: false,
            error: None,
            notice: None,
            confirmation: None,
            #[cfg(feature = "ui-preview")]
            preview: false,
        }
    }

    pub fn status(&self) -> &str {
        if self.mutating {
            "Applying service action"
        } else if self.pending.is_some() {
            "Reading configured services"
        } else if self.error.is_some() {
            "Services unavailable / refresh to retry"
        } else {
            "Local service manager"
        }
    }

    fn launch(&mut self, ctx: &egui::Context, request: Option<Request>) {
        if self.pending.is_some() {
            return;
        }
        #[cfg(feature = "ui-preview")]
        if self.preview {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let scope = self.scope;
        self.pending = Some(rx);
        self.mutating = request.is_some();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let notice = request.map(|request| services::client::apply(&request));
            let snapshot = services::client::inventory(scope);
            let _ = tx.send(Event { snapshot, notice });
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else {
            return;
        };
        match rx.try_recv() {
            Ok(event) => {
                self.pending = None;
                self.mutating = false;
                self.scanned = true;
                self.last_scan = Instant::now();
                match event.snapshot {
                    Ok(services) => {
                        self.services = services;
                        self.error = None;
                        if !self
                            .services
                            .iter()
                            .any(|s| Some(&s.id) == self.selected.as_ref())
                        {
                            self.selected = None;
                        }
                    }
                    Err(error) => self.error = Some(error),
                }
                if let Some(notice) = event.notice {
                    self.notice = Some(match notice {
                        Ok(text) => (text, false),
                        Err(text) => (text, true),
                    });
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.mutating = false;
                self.scanned = true;
                self.last_scan = Instant::now();
                self.error = Some("Service worker disconnected. Refresh to retry.".into());
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn request(&mut self, ctx: &egui::Context, request: Request) {
        if matches!(request.action, Action::Start | Action::Resume) {
            self.launch(ctx, Some(request));
        } else {
            self.confirmation = Some(request);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        self.poll();
        if !self.scanned
            || (self.auto
                && self.confirmation.is_none()
                && self.last_scan.elapsed() >= Duration::from_secs(10))
        {
            self.launch(ctx, None);
        }
        let busy = self.pending.is_some() || self.error.is_some() || self.confirmation.is_some();
        let mut request = None;
        let wide = ctx.available_rect().width() >= 1050.0;
        if let Some(service) = self
            .services
            .iter()
            .find(|s| Some(&s.id) == self.selected.as_ref())
            .cloned()
        {
            if wide {
                egui::SidePanel::right("service-inspector")
                    .exact_width(280.0)
                    .resizable(false)
                    .frame(panel(PANEL, 20))
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical()
                            .show(ui, |ui| self.details(ui, &service, busy, &mut request));
                    });
            } else {
                egui::TopBottomPanel::bottom("service-inspector-compact")
                    .exact_height(170.0)
                    .frame(panel(PANEL, 16))
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical()
                            .show(ui, |ui| self.details(ui, &service, busy, &mut request));
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(panel(BG, 28))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading("Services");
                        ui.label(
                            RichText::new("Configured services & startup behavior").color(MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(
                            ui,
                            "↻",
                            "Refresh services",
                            CYAN,
                            self.pending.is_none() && self.confirmation.is_none(),
                        )
                        .clicked()
                        {
                            self.launch(ctx, None);
                        }
                        ui.checkbox(&mut self.auto, "Auto refresh")
                            .on_hover_text("Refresh every 10 seconds");
                        if self.pending.is_some() {
                            ui.spinner();
                        }
                    });
                });
                ui.add_space(18.0);
                ui.horizontal_wrapped(|ui| {
                    let previous = self.scope;
                    ui.add_enabled_ui(
                        self.pending.is_none() && self.confirmation.is_none(),
                        |ui| {
                            for scope in [Scope::System, Scope::User] {
                                if cfg!(windows) && scope == Scope::User {
                                    continue;
                                }
                                if tab(ui, self.scope == scope, scope.label()).clicked() {
                                    self.scope = scope;
                                }
                            }
                        },
                    );
                    if previous != self.scope {
                        self.services.clear();
                        self.selected = None;
                        self.error = None;
                        self.notice = None;
                        self.scanned = false;
                        self.launch(ctx, None);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search services…")
                            .desired_width(210.0),
                    );
                    egui::ComboBox::from_id_salt("service-state")
                        .selected_text(
                            ["All states", "Running", "Stopped", "Paused", "Failed"][self.state],
                        )
                        .show_ui(ui, |ui| {
                            for (index, label) in
                                ["All states", "Running", "Stopped", "Paused", "Failed"]
                                    .iter()
                                    .enumerate()
                            {
                                ui.selectable_value(&mut self.state, index, *label);
                            }
                        });
                    egui::ComboBox::from_id_salt("service-startup")
                        .selected_text(
                            ["All startup", "Enabled", "Disabled", "Manual"][self.startup],
                        )
                        .show_ui(ui, |ui| {
                            for (index, label) in ["All startup", "Enabled", "Disabled", "Manual"]
                                .iter()
                                .enumerate()
                            {
                                ui.selectable_value(&mut self.startup, index, *label);
                            }
                        });
                });
                ui.add_space(12.0);
                if let Some(error) = &self.error {
                    ui.label(RichText::new(error).color(RED));
                }
                if let Some((notice, error)) = &self.notice {
                    let mut dismiss = false;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(notice).color(if *error { RED } else { CYAN }));
                        dismiss = icon_button(ui, "×", "Dismiss notice", MUTED, true).clicked();
                    });
                    if dismiss {
                        self.notice = None;
                    }
                }
                let query = self.query.trim().to_lowercase();
                let rows: Vec<_> = self
                    .services
                    .iter()
                    .filter(|service| {
                        let state_match = self.state == 0
                            || service.runtime()
                                == [
                                    services::RuntimeState::Other,
                                    services::RuntimeState::Running,
                                    services::RuntimeState::Stopped,
                                    services::RuntimeState::Paused,
                                    services::RuntimeState::Failed,
                                ][self.state];
                        let startup_match = match self.startup {
                            1 => service.startup_mode() == services::StartupMode::Enabled,
                            2 => matches!(
                                service.startup_mode(),
                                services::StartupMode::Disabled | services::StartupMode::Masked
                            ),
                            3 => service.startup_mode() == services::StartupMode::Manual,
                            _ => true,
                        };
                        state_match && startup_match && service.matches(&query)
                    })
                    .collect();
                ui.horizontal(|ui| {
                    caption(
                        ui,
                        &format!("{} / {} SERVICES", rows.len(), self.services.len()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        caption(ui, "STARTUP CONTROLS FUTURE LAUNCHES");
                    });
                });
                ui.add_space(8.0);
                if rows.is_empty() {
                    ui.add_space(28.0);
                    ui.label(
                        RichText::new(if self.pending.is_some() {
                            "Reading configured services…"
                        } else if self.services.is_empty() {
                            "No services loaded. Refresh to query the local service manager."
                        } else {
                            "No services match your filters."
                        })
                        .color(MUTED),
                    );
                    return;
                }
                TableBuilder::new(ui)
                    .id_salt("services-table")
                    .striped(true)
                    .resizable(false)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::remainder().at_least(120.0))
                    .column(Column::exact(100.0))
                    .column(Column::exact(100.0))
                    .column(Column::exact(if cfg!(windows) { 174.0 } else { 138.0 }))
                    .header(30.0, |mut header| {
                        for label in ["SERVICE", "STATE", "STARTUP", "ACTIONS"] {
                            header.col(|ui| caption(ui, label));
                        }
                    })
                    .body(|body| {
                        body.rows(44.0, rows.len(), |mut row| {
                            let service = rows[row.index()];
                            row.set_selected(self.selected.as_ref() == Some(&service.id));
                            row.col(|ui| {
                                let label = if service.name.is_empty() {
                                    &service.id
                                } else {
                                    &service.name
                                };
                                if ui
                                    .add(
                                        egui::Button::selectable(
                                            self.selected.as_ref() == Some(&service.id),
                                            RichText::new(label).size(13.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(format!(
                                        "{}\n{}",
                                        service.id, service.description
                                    ))
                                    .clicked()
                                {
                                    self.selected = Some(service.id.clone());
                                }
                            });
                            row.col(|ui| badge(ui, &service.state, state_color(service)));
                            row.col(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&service.startup).size(12.0).color(MUTED),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&service.startup);
                            });
                            row.col(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.spacing_mut().button_padding = egui::vec2(5.0, 4.0);
                                ui.push_id(&service.id, |ui| {
                                    let toggle = if service.allows(Action::Enable) {
                                        Action::Enable
                                    } else {
                                        Action::Disable
                                    };
                                    for action in [Action::Start, Action::Stop, Action::Restart]
                                        .into_iter()
                                        .chain(if service.manager.supports(Action::Pause) {
                                            Some(
                                                if service.runtime()
                                                    == services::RuntimeState::Paused
                                                {
                                                    Action::Resume
                                                } else {
                                                    Action::Pause
                                                },
                                            )
                                        } else {
                                            None
                                        })
                                        .chain(Some(toggle))
                                    {
                                        if action_button(ui, service, action, !busy).clicked() {
                                            request = Some(Request {
                                                scope: self.scope,
                                                service: service.clone(),
                                                action,
                                            });
                                        }
                                    }
                                });
                            });
                        });
                    });
            });
        if let Some(request) = request {
            self.request(ctx, request);
        }
        self.confirm(ctx);
    }

    fn details(
        &mut self,
        ui: &mut egui::Ui,
        service: &Service,
        busy: bool,
        request: &mut Option<Request>,
    ) {
        ui.horizontal(|ui| {
            caption(ui, "SERVICE INSPECTOR");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, "×", "Close service inspector", MUTED, true).clicked() {
                    self.selected = None;
                }
                if icon_button(ui, "⧉", "Copy service identifier", CYAN, true).clicked() {
                    ui.ctx().copy_text(service.id.clone());
                }
            });
        });
        ui.label(RichText::new(&service.name).size(20.0).color(TEXT));
        ui.horizontal_wrapped(|ui| {
            badge(ui, &service.state, state_color(service));
            badge(ui, &service.startup, MUTED);
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for action in Action::ALL {
                if !service.manager.supports(action) {
                    continue;
                }
                if action_button(ui, service, action, !busy).clicked() {
                    *request = Some(Request {
                        scope: self.scope,
                        service: service.clone(),
                        action,
                    });
                }
            }
        });
        ui.add_space(10.0);
        rule(ui);
        ui.add_space(10.0);
        field(ui, "IDENTIFIER", &service.id);
        field(ui, "SCOPE", self.scope.label());
        if let Some(pid) = service.pid {
            field(ui, "PROCESS ID", &pid.to_string());
        }
        if !service.description.is_empty() {
            field(ui, "DESCRIPTION", &service.description);
        }
        field(ui, "CONFIGURATION / COMMAND", &service.definition);
        caption(
            ui,
            "Startup controls do not start or stop a service. System changes may require administrator/root access.",
        );
    }

    fn confirm(&mut self, ctx: &egui::Context) {
        let Some(request) = self.confirmation.clone() else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::new("service-confirm")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(format!("{}?", request.action.label())).size(22.0));
            field(ui, request.scope.label(), &request.service.name);
            field(ui, "IDENTIFIER", &request.service.id);
            ui.label(request.action.explanation());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.confirmation = None;
                }
                if ui
                    .button(RichText::new(request.action.label()).color(AMBER))
                    .clicked()
                {
                    self.confirmation = None;
                    self.launch(ctx, Some(request.clone()));
                }
            });
        });
        if modal.should_close() {
            self.confirmation = None;
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, confirmation: bool, empty: bool) {
        self.preview = true;
        self.scanned = true;
        self.auto = false;
        self.services = if empty {
            Vec::new()
        } else {
            (0..36)
                .map(|i| Service {
                    manager: services::Manager::native(),
                    id: format!("vijideck.sample.{i:02}"),
                    name: [
                        "Application worker",
                        "Background scheduler",
                        "Database engine",
                        "Development gateway",
                        "Local cache",
                        "Print queue",
                    ][i % 6]
                        .into(),
                    description: "Disposable preview data. No service commands are executed."
                        .into(),
                    state: if i % 4 == 1 { "Stopped" } else { "Running" }.into(),
                    startup: if i % 4 == 1 { "Manual" } else { "Automatic" }.into(),
                    pid: if i % 4 == 1 {
                        None
                    } else {
                        Some(2100 + i as u32)
                    },
                    definition: "sample-service --foreground".into(),
                    can_stop: true,
                    can_pause: i % 3 == 0,
                    loaded: true,
                })
                .collect()
        };
        self.selected = self.services.first().map(|s| s.id.clone());
        if confirmation && let Some(service) = self.services.first() {
            self.confirmation = Some(Request {
                scope: self.scope,
                service: service.clone(),
                action: Action::Stop,
            });
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.scanned && self.pending.is_none()
    }
}

fn state_color(service: &Service) -> egui::Color32 {
    match service.runtime() {
        services::RuntimeState::Running => CYAN,
        services::RuntimeState::Failed => RED,
        services::RuntimeState::Paused => AMBER,
        _ => MUTED,
    }
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    color: egui::Color32,
    enabled: bool,
) -> egui::Response {
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new("").min_size(egui::vec2(30.0, 30.0)),
        )
        .on_hover_text(label)
        .on_disabled_hover_text(label);
    let p = ui.painter();
    let c = response.rect.center();
    let color = if enabled && ui.is_enabled() {
        color
    } else {
        MUTED.gamma_multiply(0.35)
    };
    let stroke = egui::Stroke::new(1.5_f32, color);
    let point = |x, y| c + egui::vec2(x, y);
    match icon {
        "▶" => {
            p.add(egui::Shape::convex_polygon(
                vec![point(-4.0, -6.0), point(6.0, 0.0), point(-4.0, 6.0)],
                color,
                egui::Stroke::NONE,
            ));
        }
        "■" => {
            p.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(11.0, 11.0)),
                1,
                color,
            );
        }
        "⏸" => {
            for x in [-3.5, 3.5] {
                p.line_segment(
                    [point(x, -6.0), point(x, 6.0)],
                    egui::Stroke::new(3.0_f32, color),
                );
            }
        }
        "↻" => {
            let points = (0..=24)
                .map(|i| {
                    let angle = -0.7 + i as f32 * 5.2 / 24.0;
                    c + egui::vec2(angle.cos(), angle.sin()) * 6.5
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
            p.line_segment([point(5.0, -7.0), point(5.0, -2.0)], stroke);
            p.line_segment([point(5.0, -2.0), point(0.0, -3.0)], stroke);
        }
        "✓" => {
            p.add(egui::Shape::line(
                vec![point(-6.0, 0.0), point(-2.0, 4.0), point(6.0, -5.0)],
                stroke,
            ));
        }
        "⊘" => {
            p.circle_stroke(c, 6.5, stroke);
            p.line_segment([point(-4.5, 4.5), point(4.5, -4.5)], stroke);
        }
        "◷" => {
            p.circle_stroke(c, 6.5, stroke);
            p.add(egui::Shape::line(
                vec![point(0.0, -4.0), c, point(3.0, 1.0)],
                stroke,
            ));
        }
        "⧉" => {
            for offset in [-2.0, 2.0] {
                p.rect_stroke(
                    egui::Rect::from_center_size(point(offset, offset), egui::vec2(9.0, 10.0)),
                    1,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        _ => {
            p.line_segment([point(-4.5, -4.5), point(4.5, 4.5)], stroke);
            p.line_segment([point(-4.5, 4.5), point(4.5, -4.5)], stroke);
        }
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

fn action_button(
    ui: &mut egui::Ui,
    service: &Service,
    action: Action,
    enabled: bool,
) -> egui::Response {
    let icon = match action {
        Action::Start | Action::Resume => "▶",
        Action::Stop => "■",
        Action::Restart => "↻",
        Action::Pause => "⏸",
        Action::Enable => "✓",
        Action::Disable => "⊘",
        Action::Manual => "◷",
    };
    let allowed = service.allows(action);
    let label = if allowed {
        action.label().to_string()
    } else {
        format!("{} — unavailable for this service/state", action.label())
    };
    icon_button(
        ui,
        icon,
        &label,
        if matches!(action, Action::Stop | Action::Disable) {
            AMBER
        } else {
            CYAN
        },
        enabled && allowed,
    )
}

pub fn rail_icon(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::click());
    let active = selected || response.hovered() || response.has_focus();
    let color = if active { CYAN } else { MUTED };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        5.0,
        if active {
            egui::Color32::from_rgb(17, 39, 48)
        } else {
            PANEL
        },
    );
    for y in [-8.0, 0.0, 8.0] {
        let center = rect.center() + egui::vec2(0.0, y);
        painter.line_segment(
            [
                center - egui::vec2(11.0, 0.0),
                center + egui::vec2(11.0, 0.0),
            ],
            egui::Stroke::new(1.4_f32, color),
        );
        painter.circle_filled(
            center + egui::vec2(if y == 0.0 { 5.0 } else { -5.0 }, 0.0),
            3.0,
            PANEL,
        );
        painter.circle_stroke(
            center + egui::vec2(if y == 0.0 { 5.0 } else { -5.0 }, 0.0),
            3.0,
            egui::Stroke::new(1.4_f32, color),
        );
    }
    if selected {
        painter.line_segment(
            [
                rect.left_top() + egui::vec2(0.0, 9.0),
                rect.left_bottom() - egui::vec2(0.0, 9.0),
            ],
            egui::Stroke::new(2.0_f32, CYAN),
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Services"));
    response.on_hover_text("Services")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_disconnect_clears_busy_state() {
        let mut view = Services::new();
        let (tx, rx) = mpsc::channel();
        view.pending = Some(rx);
        view.mutating = true;
        drop(tx);
        view.poll();
        assert!(view.pending.is_none());
        assert!(!view.mutating);
        assert!(view.error.is_some());
    }

    #[test]
    fn interrupting_actions_wait_for_confirmation() {
        let mut view = Services::new();
        let service = Service {
            manager: services::Manager::native(),
            id: "fixture.service".into(),
            name: "Disposable fixture".into(),
            description: String::new(),
            state: "Running".into(),
            startup: "enabled".into(),
            pid: Some(123),
            definition: String::new(),
            can_stop: true,
            can_pause: false,
            loaded: true,
        };
        for action in [
            Action::Stop,
            Action::Restart,
            Action::Pause,
            Action::Enable,
            Action::Disable,
            Action::Manual,
        ] {
            view.request(
                &egui::Context::default(),
                Request {
                    scope: Scope::System,
                    service: service.clone(),
                    action,
                },
            );
            assert!(view.pending.is_none());
            assert_eq!(view.confirmation.as_ref().unwrap().action, action);
        }
    }
}
