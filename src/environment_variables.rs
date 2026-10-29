use crate::ui::{AMBER, CYAN, LINE, MUTED, PANEL, SURFACE, rule};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct Variables {
    query: String,
    rows: Vec<(String, String, String)>,
    loaded: bool,
    view_scope: usize,
    selected: Option<(String, String)>,
    editing: bool,
    name: String,
    value: String,
    list: bool,
    items: Vec<String>,
    scope: usize,
    original: Option<(String, usize)>,
    deleting: bool,
    notice: Option<String>,
    pending: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    dragging: Option<usize>,
}
impl Variables {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            match result {
                Ok(()) => {
                    self.editing = false;
                    self.deleting = false;
                    self.refresh();
                    self.notice =
                        Some("Saved. Restart applications to receive the new settings.".into());
                }
                Err(e) => self.notice = Some(e),
            }
        }
        if !self.loaded {
            self.refresh();
        }
        if self.pending.is_some() {
            ui.disable();
        }
        ui.horizontal(|ui| {
            for (i, label) in [(0, "User"), (1, "System"), (2, "App at startup")] {
                if i == 1 && !cfg!(windows) {
                    continue;
                }
                let count = self.rows.iter().filter(|r| r.2 == scope_label(i)).count();
                if ui
                    .selectable_value(&mut self.view_scope, i, format!("{label}  {count}"))
                    .clicked()
                {
                    self.selected = None;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+ Add variable").clicked() {
                    self.name.clear();
                    self.value.clear();
                    self.items.clear();
                    self.list = false;
                    self.dragging = None;
                    self.scope = if self.view_scope == 1 { 1 } else { 0 };
                    self.original = None;
                    self.editing = true;
                    self.notice = None;
                }
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }
            });
        });
        ui.add_space(4.0);
        ui.label(RichText::new(match self.view_scope {
            1 => "Saved for all users on this computer. Changes request administrator approval.",
            2 => "Read-only snapshot received when VijiDeck launched, including shell and launcher settings. Refresh does not update this snapshot.",
            _ if cfg!(windows) => "Saved for your Windows account. New applications receive these settings.",
            _ => "Saved by VijiDeck in ~/.profile. Applies to new login shells that read this file.",
        }).size(12.0).color(MUTED));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_sized(
                [ui.available_width().min(420.0), 32.0],
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search variable names…")
                    .vertical_align(egui::Align::Center),
            );
            if !self.query.is_empty() && ui.button("Clear").clicked() {
                self.query.clear();
            }
        });
        if let Some(message) = self.notice.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(message).color(AMBER));
                if ui.small_button("Dismiss").clicked() {
                    self.notice = None;
                }
            });
        }
        ui.add_space(12.0);
        let query = self.query.trim().to_lowercase();
        let rows: Vec<_> = self
            .rows
            .iter()
            .filter(|r| r.2 == scope_label(self.view_scope) && r.0.to_lowercase().contains(&query))
            .cloned()
            .collect();
        if self
            .selected
            .as_ref()
            .is_none_or(|key| !rows.iter().any(|r| r.0 == key.0 && r.2 == key.1))
        {
            self.selected = rows.first().map(|r| (r.0.clone(), r.2.clone()));
        }
        let selected = self
            .selected
            .as_ref()
            .and_then(|key| rows.iter().find(|r| r.0 == key.0 && r.2 == key.1))
            .cloned();
        let available = ui.available_size();
        let left_width = (available.x * 0.46).max(240.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(left_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    use egui_extras::{Column, TableBuilder};
                    TableBuilder::new(ui)
                        .id_salt("variables-table")
                        .striped(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(Column::initial(left_width * 0.48))
                        .column(Column::remainder())
                        .sense(egui::Sense::click())
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                ui.label(RichText::new("Variable").color(MUTED));
                            });
                            header.col(|ui| {
                                ui.label(RichText::new("Value").color(MUTED));
                            });
                        })
                        .body(|body| {
                            body.rows(36.0, rows.len(), |mut row| {
                                let item = &rows[row.index()];
                                row.set_selected(
                                    self.selected
                                        .as_ref()
                                        .is_some_and(|k| k.0 == item.0 && k.1 == item.2),
                                );
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&item.0).size(12.0).strong(),
                                        )
                                        .truncate(),
                                    );
                                });
                                row.col(|ui| {
                                    let preview = if is_list(&item.0) {
                                        format!("{} entries", split_list(&item.1).len())
                                    } else {
                                        item.1.replace(['\r', '\n'], " ")
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(preview).size(12.0).color(MUTED),
                                        )
                                        .truncate(),
                                    );
                                });
                                if row.response().clicked() {
                                    self.selected = Some((item.0.clone(), item.2.clone()));
                                }
                            });
                        });
                    if rows.is_empty() {
                        ui.add_space(24.0);
                        ui.label(if self.query.is_empty() {
                            "No saved variables in this scope."
                        } else {
                            "No variables match your search."
                        });
                    }
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    if let Some((name, value, scope)) = selected {
                        ui.add(
                            egui::Label::new(RichText::new(&name).size(15.0).strong().color(CYAN))
                                .wrap(),
                        );
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if scope == "Inherited" {
                                    "App at startup · read only"
                                } else {
                                    &scope
                                })
                                .size(12.0)
                                .color(MUTED),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Copy").clicked() {
                                        ui.ctx().copy_text(value.clone());
                                    }
                                    if scope != "Inherited" && ui.button("Edit").clicked() {
                                        self.begin_edit(&name, &value, &scope);
                                    }
                                    if scope != "Inherited" && ui.button("Delete").clicked() {
                                        self.begin_edit(&name, &value, &scope);
                                        self.editing = false;
                                        self.deleting = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(10.0);
                        rule(ui);
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("variable-detail")
                            .show(ui, |ui| {
                                if is_list(&name) {
                                    let items = split_list(&value);
                                    ui.label(
                                        RichText::new(format!(
                                            "{} entries · in stored order",
                                            items.len()
                                        ))
                                        .size(12.0)
                                        .color(MUTED),
                                    );
                                    for (i, item) in items.iter().enumerate() {
                                        egui::Frame::new()
                                            .fill(if i % 2 == 0 { SURFACE } else { PANEL })
                                            .inner_margin(10)
                                            .show(ui, |ui| {
                                                ui.set_min_width(ui.available_width());
                                                ui.horizontal_top(|ui| {
                                                    ui.label(
                                                        RichText::new(format!("{:02}", i + 1))
                                                            .size(11.0)
                                                            .color(MUTED),
                                                    );
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(if item.is_empty() {
                                                                "(empty entry)"
                                                            } else {
                                                                item
                                                            })
                                                            .size(12.0),
                                                        )
                                                        .wrap()
                                                        .selectable(true),
                                                    );
                                                });
                                            });
                                    }
                                } else {
                                    ui.add(
                                        egui::Label::new(RichText::new(&value).size(12.0))
                                            .wrap()
                                            .selectable(true),
                                    );
                                }
                            });
                    } else {
                        ui.label(
                            RichText::new("Select a variable to inspect its value.").color(MUTED),
                        );
                    }
                },
            );
        });
        self.dialogs(ui.ctx());
    }
    fn begin_edit(&mut self, name: &str, value: &str, scope: &str) {
        self.name = name.into();
        self.value = value.into();
        self.scope = if scope == "System" { 1 } else { 0 };
        self.list = is_list(name);
        self.items = split_list(value);
        self.original = Some((name.into(), self.scope));
        self.editing = true;
        self.notice = None;
        self.dragging = None;
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.editing {
            let viewport = ctx.content_rect();
            let width = (viewport.width() - 64.0).clamp(280.0, 680.0);
            let list_height = (viewport.height() - 390.0).clamp(90.0, 300.0);
            egui::Window::new(if self.original.is_some() { "Edit variable" } else { "Add variable" }).id(egui::Id::new("variable-editor")).default_width(width).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.set_width(width);
                if self.pending.is_some() { ui.spinner(); ui.label("Saving changes…"); ui.disable(); }
                ui.label("Variable name");
                let response = ui.add_enabled(self.original.is_none(), egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY).font(egui::FontId::proportional(12.0)).vertical_align(egui::Align::Center));
                if response.changed() && is_list(&self.name) && !self.list { self.list = true; self.items = split_list(&self.value); }
                if cfg!(windows) && self.original.is_none() { ui.horizontal(|ui| { ui.selectable_value(&mut self.scope, 0, "User"); ui.selectable_value(&mut self.scope, 1, "System"); }); }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Value");
                    let previous = self.list;
                    ui.selectable_value(&mut self.list, false, "Text"); ui.selectable_value(&mut self.list, true, "List");
                    if previous != self.list { if self.list { self.items = split_list(&self.value); } else { self.value = join_list(&self.items); } }
                });
                if self.list {
                    ui.label(RichText::new("Drag the handle to change the order.").size(12.0).color(MUTED));
                    let mut remove = None;
                    let mut drop_index = None;
                    let mut drop_y = None;
                    egui::ScrollArea::vertical().id_salt("variable-list-editor").max_height(list_height).auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(ui, |ui| {
                        for (i, item) in self.items.iter_mut().enumerate() {
                            ui.push_id(i, |ui| {
                                let row_width = (ui.available_width() - 12.0).max(0.0);
                                let spacing = ui.spacing().item_spacing.x;
                                let number_width = 30.0;
                                let button_width = 30.0;
                                let field_width = (row_width - number_width - 2.0 * button_width - 3.0 * spacing).max(1.0);
                                let row = ui.horizontal(|ui| {
                                    let drag = drag_handle(ui).on_hover_text("Drag to reorder");
                                    if drag.drag_started() { self.dragging = Some(i); }
                                    ui.add_sized([number_width, 30.0], egui::Label::new(RichText::new(format!("{:02}", i + 1)).size(11.0).color(MUTED)));
                                    ui.add_sized([field_width, 30.0], egui::TextEdit::singleline(item).desired_width(field_width).font(egui::FontId::proportional(12.0)).vertical_align(egui::Align::Center));
                                    if remove_button(ui).on_hover_text("Remove entry").clicked() { remove = Some(i); }
                                });
                                if self.dragging.is_some() && let Some(pointer) = ui.ctx().pointer_interact_pos() {
                                    let rect = row.response.rect;
                                    if rect.intersect(ui.clip_rect()).contains(pointer) {
                                        let after = pointer.y >= rect.center().y;
                                        drop_index = Some(i + usize::from(after));
                                        drop_y = Some(if after { rect.bottom() } else { rect.top() });
                                        let clip = ui.clip_rect();
                                        if pointer.y < clip.top() + 24.0 { ui.scroll_to_rect(egui::Rect::from_min_max(rect.min - egui::vec2(0.0, 45.0), rect.max), None); }
                                        if pointer.y > clip.bottom() - 24.0 { ui.scroll_to_rect(egui::Rect::from_min_max(rect.min, rect.max + egui::vec2(0.0, 45.0)), None); }
                                    }
                                }
                            });
                        }
                        if let Some(y) = drop_y { let clip = ui.clip_rect(); ui.painter().line_segment([egui::pos2(clip.left(), y), egui::pos2(clip.right() - 12.0, y)], egui::Stroke::new(2.0_f32, CYAN)); }
                    });
                    if let Some(i) = remove { self.items.remove(i); self.dragging = None; }
                    if self.dragging.is_some() && ctx.input(|input| input.pointer.any_released()) {
                        if let (Some(from), Some(target)) = (self.dragging, drop_index) { move_entry(&mut self.items, from, target); }
                        self.dragging = None;
                    }
                    ui.add_space(4.0);
                    if ui.button("+ Add entry").clicked() { self.items.push(String::new()); }
                } else { ui.add(egui::TextEdit::multiline(&mut self.value).desired_width(f32::INFINITY).desired_rows(5).font(egui::FontId::proportional(12.0)).vertical_align(egui::Align::Center)); }
                ui.add_space(10.0); rule(ui);
                ui.label(RichText::new(if self.scope == 1 { "Saving requests administrator approval. Restart applications to receive changes." } else { "Restart applications to receive changes." }).size(12.0).color(MUTED));
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { self.editing = false; self.dragging = None; }
                    if ui.add_enabled(valid_name(&self.name), egui::Button::new("Save changes").fill(CYAN.gamma_multiply(0.2))).clicked() {
                        if self.list { self.value = join_list(&self.items); }
                        self.apply(Some(self.value.clone()), ctx);
                    }
                });
                if let Some(message) = &self.notice { ui.label(RichText::new(message).color(AMBER)); }
            });
        }
        if self.deleting {
            egui::Window::new("Delete variable?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    if self.pending.is_some() {
                        ui.spinner();
                        ui.label("Saving changes…");
                        ui.disable();
                    }
                    ui.label(format!(
                        "Remove {} from {} settings?",
                        self.name,
                        scope_label(self.scope)
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.deleting = false;
                        }
                        if ui.button("Delete variable").clicked() {
                            self.apply(None, ctx);
                        }
                    });
                    if let Some(message) = &self.notice {
                        ui.label(RichText::new(message).color(AMBER));
                    }
                });
        }
    }
    fn apply(&mut self, value: Option<String>, ctx: &egui::Context) {
        self.notice = None;
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        let name = self.name.clone();
        let scope = self.scope;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(save(&name, value.as_deref(), scope));
            ctx.request_repaint();
        });
    }
    fn refresh(&mut self) {
        self.loaded = true;
        self.rows = std::env::vars_os()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.to_string_lossy().into_owned(),
                    "Inherited".into(),
                )
            })
            .collect();
        match persistent() {
            Ok(rows) => self.rows.extend(rows),
            Err(e) => self.notice = Some(e),
        }
        self.rows.sort_by_key(|r| (r.0.to_lowercase(), r.2.clone()));
        #[cfg(feature = "ui-preview")]
        if std::env::var_os("VIJIDECK_PREVIEW_VARIABLES").is_some() {
            self.view_scope = if cfg!(windows) { 1 } else { 2 };
            self.selected = self
                .rows
                .iter()
                .find(|r| r.0.eq_ignore_ascii_case("PATH") && r.2 == scope_label(self.view_scope))
                .map(|r| (r.0.clone(), r.2.clone()));
            if std::env::var_os("VIJIDECK_PREVIEW_VARIABLE_EDITOR").is_some() {
                self.begin_edit(
                    "PATH",
                    &join_list(
                        &(1..=15)
                            .map(|i| format!("C:\\Example\\Tools\\Version {i}\\bin"))
                            .collect::<Vec<_>>(),
                    ),
                    "User",
                );
            }
        }
    }
}
fn move_entry<T>(items: &mut Vec<T>, from: usize, insertion: usize) {
    if from >= items.len() || insertion > items.len() {
        return;
    }
    let target = if insertion > from {
        insertion - 1
    } else {
        insertion
    };
    if from != target {
        let item = items.remove(from);
        items.insert(target, item);
    }
}

fn drag_handle(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::drag());
    let response = response.on_hover_cursor(egui::CursorIcon::Grab);
    let color = if response.hovered() || response.dragged() {
        CYAN
    } else {
        MUTED
    };
    let painter = ui.painter();
    if response.hovered() || response.dragged() {
        painter.rect_filled(rect, 4.0, LINE);
    }
    for x in [-3.0, 3.0] {
        for y in [-5.0, 0.0, 5.0] {
            painter.circle_filled(rect.center() + egui::vec2(x, y), 1.2, color);
        }
    }
    response
}

fn remove_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, if response.hovered() { LINE } else { SURFACE });
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0_f32, LINE),
        egui::StrokeKind::Inside,
    );
    let c = rect.center();
    let stroke = egui::Stroke::new(1.5_f32, if response.hovered() { CYAN } else { MUTED });
    painter.line_segment(
        [c + egui::vec2(-4.0, -4.0), c + egui::vec2(4.0, 4.0)],
        stroke,
    );
    painter.line_segment(
        [c + egui::vec2(-4.0, 4.0), c + egui::vec2(4.0, -4.0)],
        stroke,
    );
    response
}
fn scope_label(scope: usize) -> &'static str {
    match scope {
        1 => "System",
        2 => "Inherited",
        _ => "User",
    }
}
fn is_list(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    matches!(
        n.as_str(),
        "PATH"
            | "PATHEXT"
            | "CLASSPATH"
            | "PSMODULEPATH"
            | "PYTHONPATH"
            | "NODE_PATH"
            | "LD_LIBRARY_PATH"
            | "DYLD_LIBRARY_PATH"
            | "MANPATH"
            | "INFOPATH"
            | "CPATH"
            | "LIBRARY_PATH"
            | "PKG_CONFIG_PATH"
            | "CMAKE_PREFIX_PATH"
    )
}
fn split_list(value: &str) -> Vec<String> {
    value
        .split(if cfg!(windows) { ';' } else { ':' })
        .map(str::to_owned)
        .collect()
}
fn join_list(items: &[String]) -> String {
    items.join(if cfg!(windows) { ";" } else { ":" })
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

#[cfg(windows)]
fn powershell(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
#[cfg(windows)]
fn persistent() -> Result<Vec<(String, String, String)>, String> {
    let out = powershell(
        r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8; $r = @(); foreach ($s in @('User','Machine')) { $target = [EnvironmentVariableTarget]::$s; foreach ($e in [Environment]::GetEnvironmentVariables($target).GetEnumerator()) { $r += @{name=[string]$e.Key; value=[string]$e.Value; scope=if($s -eq 'Machine'){'System'}else{'User'}} } }; ConvertTo-Json -Compress -InputObject @($r)"#,
    )?;
    let data: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    Ok(data
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            Some((
                v["name"].as_str()?.into(),
                v["value"].as_str()?.into(),
                v["scope"].as_str()?.into(),
            ))
        })
        .collect())
}
#[cfg(windows)]
fn save(name: &str, value: Option<&str>, scope: usize) -> Result<(), String> {
    if !valid_name(name) || value.is_some_and(|v| v.is_empty() || v.contains('\0')) {
        return Err(
            "Use a valid variable name and a nonempty value without NUL characters.".into(),
        );
    }
    // Base64 encodes UTF-16 PowerShell source, so values are never interpreted as commands.
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let expression = value.map(quote).unwrap_or_else(|| "$null".into());
    let script = format!(
        "try {{ [Environment]::SetEnvironmentVariable({}, {}, '{}'); exit 0 }} catch {{ exit 1 }}",
        quote(name),
        expression,
        if scope == 1 { "Machine" } else { "User" }
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64(&bytes);
    if scope == 1 {
        powershell(&format!(
            "$p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile -NonInteractive -EncodedCommand {encoded}'; if ($p.ExitCode -ne 0) {{ throw 'The elevated change failed.' }}"
        ))?;
    } else {
        powershell(&format!(
            "& powershell.exe -NoProfile -NonInteractive -EncodedCommand {encoded}; if ($LASTEXITCODE -ne 0) {{ throw 'The change failed.' }}"
        ))?;
    }
    Ok(())
}
#[cfg(windows)]
fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        s.push(T[((n >> 18) & 63) as usize] as char);
        s.push(T[((n >> 12) & 63) as usize] as char);
        s.push(if c.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}

#[cfg(not(windows))]
fn profile() -> Result<std::path::PathBuf, String> {
    Ok(
        std::path::PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?)
            .join(".profile"),
    )
}
#[cfg(not(windows))]
fn persistent() -> Result<Vec<(String, String, String)>, String> {
    let path = profile()?;
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    Ok(content
        .lines()
        .filter_map(|line| {
            let (name, encoded) = line.strip_prefix("# VijiDeck variable ")?.split_once('=')?;
            let bytes: Option<Vec<u8>> = encoded
                .as_bytes()
                .chunks(2)
                .map(|c| {
                    if c.len() != 2 {
                        return None;
                    }
                    u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()
                })
                .collect();
            Some((name.into(), String::from_utf8(bytes?).ok()?, "User".into()))
        })
        .collect())
}
#[cfg(not(windows))]
fn save(name: &str, value: Option<&str>, _: usize) -> Result<(), String> {
    if !valid_name(name) || value.is_some_and(|v| v.contains(['\0', '\n', '\r'])) {
        return Err("Use a valid shell variable name and a single-line value.".into());
    }
    let path = profile()?;
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let marker = format!("# VijiDeck variable {name}=");
    let export = format!("export {name}=");
    let mut lines = content.lines();
    let mut result = String::new();
    while let Some(line) = lines.next() {
        if line.starts_with(&marker) {
            if let Some(next) = lines.next() {
                if !next.starts_with(&export) {
                    result.push_str(next);
                    result.push('\n');
                }
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    if let Some(v) = value {
        let hex: String = v.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        result.push_str(&format!(
            "{marker}{hex}\nexport {name}='{}'\n",
            v.replace('\'', "'\\''")
        ));
    }
    std::fs::write(path, result).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_entries_uses_the_drop_gap_and_preserves_duplicates() {
        let mut items = vec!["a", "b", "a", "c"];
        move_entry(&mut items, 0, 4);
        assert_eq!(items, ["b", "a", "c", "a"]);
        move_entry(&mut items, 3, 1);
        assert_eq!(items, ["b", "a", "a", "c"]);
        move_entry(&mut items, 1, 2);
        assert_eq!(items, ["b", "a", "a", "c"]);
    }
    #[test]
    fn path_list_round_trip_preserves_order_duplicates_and_empty_entries() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        let value = ["first path", "", "first path", "%JAVA_HOME%", ""].join(separator);
        assert_eq!(join_list(&split_list(&value)), value);
        assert_eq!(split_list(&value).len(), 5);
    }
    #[test]
    fn recognizes_search_lists_without_splitting_single_directory_variables() {
        for name in [
            "Path",
            "PATHEXT",
            "PYTHONPATH",
            "PSModulePath",
            "LD_LIBRARY_PATH",
        ] {
            assert!(is_list(name));
        }
        for name in ["JAVA_HOME", "CUDA_PATH", "HOME", "TEMP"] {
            assert!(!is_list(name));
        }
    }
}
