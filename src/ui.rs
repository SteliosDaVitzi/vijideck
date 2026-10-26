use eframe::egui::{self, Color32, RichText, Stroke, Vec2};

pub const BG: Color32 = Color32::from_rgb(8, 13, 18);
pub const PANEL: Color32 = Color32::from_rgb(11, 18, 24);
pub const SURFACE: Color32 = Color32::from_rgb(16, 26, 34);
pub const LINE: Color32 = Color32::from_rgb(31, 49, 60);
pub const TEXT: Color32 = Color32::from_rgb(219, 233, 239);
pub const MUTED: Color32 = Color32::from_rgb(133, 158, 172);
pub const CYAN: Color32 = Color32::from_rgb(84, 218, 235);
pub const AMBER: Color32 = Color32::from_rgb(228, 182, 110);
pub const RED: Color32 = Color32::from_rgb(248, 125, 129);

pub fn theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(12, 20, 27);
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = 8.into();
    v.selection.bg_fill = Color32::from_rgb(19, 54, 65);
    v.selection.stroke = Stroke::new(1.0, CYAN);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.inactive.bg_fill = SURFACE;
    v.widgets.inactive.weak_bg_fill = SURFACE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = Color32::from_rgb(22, 42, 52);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(22, 42, 52);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, CYAN.gamma_multiply(0.6));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, CYAN);
    v.widgets.active.bg_fill = Color32::from_rgb(22, 57, 68);
    v.widgets.active.weak_bg_fill = v.widgets.active.bg_fill;
    v.widgets.active.bg_stroke = Stroke::new(1.0, CYAN);
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        widget.corner_radius = 4.into();
    }
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.scroll.bar_width = 5.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(30.0));
    ctx.set_style(style);
}

pub fn panel(fill: Color32, margin: i8) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(margin)
}

pub fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(MUTED));
}

pub fn rule(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, LINE));
}

pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 2.5, color);
}

pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.25)))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(text).monospace().size(10.0).color(color)).extend(),
            );
        });
}

pub fn field(ui: &mut egui::Ui, label: &str, value: &str) {
    caption(ui, label);
    ui.add(
        egui::Label::new(RichText::new(value).size(13.0))
            .wrap()
            .selectable(true),
    );
    ui.add_space(5.0);
}

pub fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    caption(ui, label);
    ui.label(RichText::new(value).size(23.0).color(TEXT));
}

pub fn tab(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let response = ui.add(
        egui::Button::new(RichText::new(text).color(if selected { CYAN } else { MUTED }))
            .fill(if selected {
                Color32::from_rgb(17, 39, 48)
            } else {
                Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE),
    );
    if selected {
        ui.painter().hline(
            response.rect.x_range(),
            response.rect.bottom(),
            Stroke::new(1.5, CYAN),
        );
    }
    response
}

pub fn reactor(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let center = rect.center();
    let p = ui.painter();
    p.circle_stroke(center, size * 0.39, Stroke::new(1.0, LINE));
    for i in 0..3 {
        let start = i as f32 * std::f32::consts::TAU / 3.0 - 1.4;
        let points: Vec<_> = (0..=20)
            .map(|n| center + Vec2::angled(start + n as f32 / 20.0 * 1.55) * size * 0.30)
            .collect();
        p.add(egui::Shape::line(points, Stroke::new(2.0, CYAN)));
    }
    p.circle_filled(center, size * 0.09, CYAN);
}

pub fn port_icon(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(
        rect,
        5.0,
        if selected {
            Color32::from_rgb(17, 39, 48)
        } else {
            PANEL
        },
    );
    if selected {
        p.line_segment(
            [
                rect.left_top() + egui::vec2(0.0, 9.0),
                rect.left_bottom() - egui::vec2(0.0, 9.0),
            ],
            Stroke::new(2.0, CYAN),
        );
    }
    let c = rect.center();
    for x in [-8.0, 0.0, 8.0] {
        p.line_segment(
            [c + egui::vec2(x, -7.0), c + egui::vec2(x, 7.0)],
            Stroke::new(1.0, CYAN),
        );
        p.circle_filled(
            c + egui::vec2(x, if x == 0.0 { 3.0 } else { -2.0 }),
            2.5,
            PANEL,
        );
        p.circle_stroke(
            c + egui::vec2(x, if x == 0.0 { 3.0 } else { -2.0 }),
            2.5,
            Stroke::new(1.0, CYAN),
        );
    }
    response.on_hover_text("Port manager")
}

pub fn disk_glyph(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let r = rect.shrink(rect.width() * 0.12);
    p.rect_stroke(r, 3.0, Stroke::new(1.3, color), egui::StrokeKind::Inside);
    let y = r.bottom() - r.height() * 0.25;
    p.line_segment(
        [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
        Stroke::new(1.2, color),
    );
    p.circle_stroke(
        egui::pos2(r.center().x, r.top() + r.height() * 0.35),
        r.width() * 0.17,
        Stroke::new(1.1, color),
    );
    p.circle_filled(egui::pos2(r.right() - 4.0, r.bottom() - 3.5), 1.2, color);
}

pub fn cleanup_icon(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::click());
    let active = selected || response.hovered() || response.has_focus();
    let p = ui.painter();
    p.rect_filled(
        rect,
        5.0,
        if active {
            Color32::from_rgb(17, 39, 48)
        } else {
            PANEL
        },
    );
    if selected {
        p.line_segment(
            [
                rect.left_top() + egui::vec2(0.0, 9.0),
                rect.left_bottom() - egui::vec2(0.0, 9.0),
            ],
            Stroke::new(2.0, CYAN),
        );
    }
    disk_glyph(p, rect.shrink(8.0), if active { CYAN } else { MUTED });
    response.on_hover_text("Disk cleanup")
}

pub fn logs_icon(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::click());
    let active = selected || response.hovered() || response.has_focus();
    let color = if active { CYAN } else { MUTED };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        5.0,
        if active {
            Color32::from_rgb(17, 39, 48)
        } else {
            PANEL
        },
    );
    if selected {
        painter.line_segment(
            [
                rect.left_top() + egui::vec2(0.0, 9.0),
                rect.left_bottom() - egui::vec2(0.0, 9.0),
            ],
            Stroke::new(2.0, CYAN),
        );
    }
    let page = rect.shrink(13.0);
    painter.rect_stroke(page, 2.0, Stroke::new(1.3, color), egui::StrokeKind::Inside);
    for y in [-4.0, 0.0, 4.0] {
        painter.line_segment(
            [
                egui::pos2(page.left() + 4.0, page.center().y + y),
                egui::pos2(page.right() - 4.0, page.center().y + y),
            ],
            Stroke::new(1.2, color),
        );
    }
    response.on_hover_text("System logs")
}

pub fn tasks_icon(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::click());
    let active = selected || response.hovered() || response.has_focus();
    let p = ui.painter();
    p.rect_filled(
        rect,
        5.0,
        if active {
            Color32::from_rgb(17, 39, 48)
        } else {
            PANEL
        },
    );
    if selected {
        p.line_segment(
            [
                rect.left_top() + egui::vec2(0.0, 9.0),
                rect.left_bottom() - egui::vec2(0.0, 9.0),
            ],
            Stroke::new(2.0, CYAN),
        );
    }
    let c = rect.center();
    let color = if active { CYAN } else { MUTED };
    p.rect_stroke(
        rect.shrink(11.0),
        2.0,
        Stroke::new(1.3, color),
        egui::StrokeKind::Inside,
    );
    p.add(egui::Shape::line(
        vec![
            c + egui::vec2(-8.0, 2.0),
            c + egui::vec2(-4.0, 2.0),
            c + egui::vec2(-1.0, -6.0),
            c + egui::vec2(2.0, 6.0),
            c + egui::vec2(5.0, -1.0),
            c + egui::vec2(8.0, -1.0),
        ],
        Stroke::new(1.3, color),
    ));
    response.on_hover_text("Process manager")
}

pub fn port_heading(ui: &mut egui::Ui, port: u16, protocol: &str, state: &str, compact: bool) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), if compact { 80.0 } else { 100.0 }),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    for (corner, dx, dy) in [
        (rect.left_top(), 1.0, 1.0),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        painter.line_segment(
            [corner + egui::vec2(0.0, dy * 14.0), corner],
            Stroke::new(1.0, CYAN.gamma_multiply(0.45)),
        );
        painter.line_segment(
            [corner, corner + egui::vec2(dx * 14.0, 0.0)],
            Stroke::new(1.0, CYAN.gamma_multiply(0.45)),
        );
    }
    painter.text(
        rect.left_top() + egui::vec2(16.0, 15.0),
        egui::Align2::LEFT_TOP,
        "SELECTED PORT",
        egui::FontId::monospace(10.0),
        MUTED,
    );
    painter.text(
        rect.left_top() + egui::vec2(14.0, 33.0),
        egui::Align2::LEFT_TOP,
        port.to_string(),
        egui::FontId::proportional(if compact { 35.0 } else { 46.0 }),
        CYAN,
    );
    painter.text(
        rect.right_bottom() - egui::vec2(14.0, 18.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{protocol} / {state}"),
        egui::FontId::monospace(11.0),
        MUTED,
    );
}
