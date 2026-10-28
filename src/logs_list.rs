use crate::{logs::Entry, ui::*};
use eframe::egui::{self, Color32, Stroke};

const ROW_HEIGHT: f32 = 48.0;
const SELECTED: Color32 = Color32::from_rgb(17, 39, 48);

#[derive(Default)]
pub struct LogList {
    scroll_y: f32,
    #[cfg(test)]
    pub visible_rows: Vec<(usize, egui::Rect)>,
}

impl LogList {
    #[cfg(test)]
    pub fn scroll_offset(&self) -> f32 {
        self.scroll_y
    }

    pub fn reset(&mut self) {
        self.scroll_y = 0.0;
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        entries: &[Entry],
        indices: &[usize],
        selected: Option<usize>,
    ) -> Option<usize> {
        #[cfg(test)]
        self.visible_rows.clear();
        let mut clicked = None;
        let area = egui::ScrollArea::vertical()
            .id_salt("system_log_entries_v2")
            .auto_shrink([false, false])
            .vertical_scroll_offset(self.scroll_y)
            .scroll_source(egui::scroll_area::ScrollSource {
                scroll_bar: true,
                mouse_wheel: true,
                drag: false,
            })
            .show(ui, |ui| {
                let width = ui.available_width();
                // Allocate every entry in order. No virtual row offsets or invisible buttons.
                for &index in indices {
                    let entry = &entries[index];
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
                    let response = ui
                        .interact(
                            rect,
                            ui.make_persistent_id(("system_log_entry", index)),
                            // Pointer selection must not introduce keyboard focus/scroll requests.
                            egui::Sense::CLICK,
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        clicked = Some(index);
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            true,
                            selected == Some(index),
                            &entry.source,
                        )
                    });
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    #[cfg(test)]
                    self.visible_rows.push((index, rect));
                    let is_selected = selected == Some(index);
                    ui.painter().rect(
                        rect,
                        4.0,
                        if is_selected { SELECTED } else { SURFACE },
                        Stroke::new(1.0_f32, if is_selected { CYAN } else { LINE }),
                        egui::StrokeKind::Inside,
                    );
                    let level_color = match entry.level.as_str() {
                        "Critical" | "Error" => RED,
                        "Warning" => AMBER,
                        _ => MUTED,
                    };
                    let painter = ui.painter().with_clip_rect(
                        ui.clip_rect()
                            .intersect(rect.shrink2(egui::vec2(10.0, 3.0))),
                    );
                    for (text, y, color) in [
                        (
                            format!("{}  ·  {}  ·  {}", entry.time, entry.level, entry.source),
                            7.0,
                            level_color,
                        ),
                        (
                            entry
                                .message
                                .lines()
                                .next()
                                .unwrap_or("(no message)")
                                .to_owned(),
                            25.0,
                            TEXT,
                        ),
                    ] {
                        painter.text(
                            rect.left_top() + egui::vec2(10.0, y),
                            egui::Align2::LEFT_TOP,
                            text,
                            egui::FontId::proportional(12.0),
                            color,
                        );
                    }
                }
            });
        self.scroll_y = area.state.offset.y;
        clicked
    }
}
