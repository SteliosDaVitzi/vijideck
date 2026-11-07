use crate::environment_settings::{save, valid_name};
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
    loading: Option<std::sync::mpsc::Receiver<Result<crate::environment_settings::Rows, String>>>,
}
impl Variables {
    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.loaded && self.loading.is_none()
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.loading.as_ref().and_then(crate::worker::poll) {
            self.loading = None;
            match result {
                Ok(rows) => self.rows.extend(rows),
                Err(e) => self.notice = Some(e),
            }
            self.rows.sort_by_key(|r| (r.0.to_lowercase(), r.2.clone()));
        }
        if let Some(result) = self.pending.as_ref().and_then(crate::worker::poll) {
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
        if self.loading.is_some() {
            ui.spinner();
            ui.label("Reading saved settings…");
        }
        if self.pending.is_some() || self.loading.is_some() {
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
                if self.pending.is_some() || self.loading.is_some() { ui.spinner(); ui.label("Saving changes…"); ui.disable(); }
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
        if self.pending.is_some() || self.loading.is_some() {
            return;
        }
        self.notice = None;

        let name = self.name.clone();
        let scope = self.scope;
        let ctx = ctx.clone();
        self.pending = Some(crate::worker::spawn(move || {
            let result = save(&name, value.as_deref(), scope);
            ctx.request_repaint();
            result
        }));
    }
    fn refresh(&mut self) {
        if self.loading.is_some() {
            return;
        }
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
        self.loading = Some(crate::worker::spawn(
            crate::environment_settings::persistent,
        ));
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_settings_jobs_release_the_view() {
        let ctx = egui::Context::default();
        let (tx, loading) = std::sync::mpsc::channel();
        let (save_tx, pending) = std::sync::mpsc::channel();
        let mut view = Variables {
            loaded: true,
            loading: Some(loading),
            pending: Some(pending),
            ..Default::default()
        };
        drop(tx);
        drop(save_tx);
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| view.show(ui));
        });
        assert!(view.loading.is_none() && view.pending.is_none());
        assert!(
            view.notice
                .as_ref()
                .is_some_and(|notice| notice.contains("Worker stopped"))
        );
    }
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
