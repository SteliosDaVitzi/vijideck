//! Port-manager rendering and confirmation UI, owned by the application shell.
use super::*;
use egui_extras::{Column, TableBuilder};

impl VijiDeck {
    pub(super) fn show_ports(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("process_inspector")
            .default_width(350.0)
            .width_range(310.0..=(ctx.content_rect().width() - 548.0).clamp(310.0, 500.0))
            .frame(panel(PANEL, 22))
            .show(ctx, |ui| self.inspector(ui));
        egui::CentralPanel::default()
            .frame(panel(BG, 28))
            .show(ctx, |ui| self.main_panel(ui));
        self.confirm_stop(ctx);
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
