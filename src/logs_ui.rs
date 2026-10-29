use crate::{logs, ui::*};
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, TimeZone, Timelike};
use eframe::egui::{self, RichText};
use std::sync::mpsc::{self, Receiver};

pub struct Logs {
    opened: bool,
    source: usize,
    minutes: u32,
    custom_range: bool,
    range_open: bool,
    range_error: Option<String>,
    start_date: NaiveDate,
    end_date: NaiveDate,
    calendar_month: NaiveDate,
    choosing_end: bool,
    start_hour: u32,
    end_hour: u32,
    query: String,
    level: String,
    snapshot: Option<logs::Snapshot>,
    selected: Option<usize>,
    pending: Option<Receiver<Result<logs::Snapshot, logs::ReadError>>>,
    access_pending: Option<Receiver<Result<(), String>>>,
    access_notice: Option<String>,
    administrator_window: bool,
    elevated_reads: bool,
    error: Option<logs::ReadError>,
    list: crate::logs_list::LogList,
}
impl Logs {
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self) {
        self.opened = true;
        self.selected = Some(3);
        self.snapshot = Some(logs::Snapshot {
            note: "Sample events for UI preview. Search filters loaded entries.".into(),
            access_limited: false,
            entries: (0..80)
                .map(|i| {
                    logs::Entry::test_entry(
                        format!("2026-10-26 14:{:02}:00", 59 - i % 60),
                        if i % 7 == 0 { "Warning" } else { "Info" }.into(),
                        format!("Sample system service / {}", 7000 + i),
                        format!("Sample event {i}: service state updated successfully."),
                        format!("Sample event {i}\nService state: Running\nThis is preview data."),
                        format!("sample event {i}"),
                    )
                })
                .collect(),
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.opened && self.pending.is_none()
    }
    pub fn new() -> Self {
        Self {
            opened: false,
            source: logs::default_source(),
            minutes: 60,
            custom_range: false,
            range_open: false,
            range_error: None,
            start_date: Local::now().date_naive(),
            end_date: Local::now().date_naive(),
            calendar_month: Local::now().date_naive().with_day(1).unwrap(),
            choosing_end: false,
            start_hour: Local::now().hour().saturating_sub(1),
            end_hour: Local::now().hour(),
            query: String::new(),
            level: "All".into(),
            snapshot: None,
            selected: None,
            pending: None,
            access_pending: None,
            access_notice: None,
            administrator_window: false,
            elevated_reads: false,
            error: None,
            list: crate::logs_list::LogList::default(),
        }
    }
    pub fn configure_startup(&mut self, request: crate::logs_access::Request) {
        self.source = request.source;
        self.minutes = request.minutes;
        if let Some((start, end)) = request.range
            && let (Ok(start), Ok(end)) = (
                chrono::DateTime::parse_from_rfc3339(&start),
                chrono::DateTime::parse_from_rfc3339(&end),
            )
        {
            let start = start.with_timezone(&Local);
            let end = end.with_timezone(&Local);
            self.start_date = start.date_naive();
            self.start_hour = start.hour();
            self.end_date = end.date_naive();
            self.end_hour = end.hour();
            self.custom_range = true;
        }
        #[cfg(windows)]
        {
            self.administrator_window =
                unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 };
        }
    }

    fn request_access(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.access_pending.is_some() {
            return;
        }
        #[cfg(windows)]
        {
            let request = crate::logs_access::Request {
                source: self.source,
                minutes: self.minutes,
                range: if self.custom_range {
                    self.range_bounds()
                } else {
                    None
                },
            };
            let (tx, rx) = mpsc::channel();
            self.access_pending = Some(rx);
            self.access_notice = None;
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(crate::logs_access::open_administrator_window(request));
                ctx.request_repaint();
            });
        }
        #[cfg(not(windows))]
        {
            self.elevated_reads = true;
            self.refresh(ctx);
        }
    }
    pub fn status(&self) -> &str {
        if self.pending.is_some() {
            "Reading system logs"
        } else {
            "System logs / read only"
        }
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let range = if self.custom_range {
            let Some(range) = self.range_bounds() else {
                self.range_error = Some("Choose a valid local time range.".into());
                self.range_open = true;
                return;
            };
            Some(range)
        } else {
            None
        };
        let (tx, rx) = mpsc::channel();
        let source = self.source;
        let minutes = self.minutes;
        let elevated = self.elevated_reads;
        let ctx = ctx.clone();
        self.pending = Some(rx);
        self.error = None;
        self.snapshot = None;
        self.selected = None;
        self.list.reset();
        std::thread::spawn(move || {
            let result = if elevated {
                logs::load_with_access(source, minutes, range, true)
            } else {
                logs::load(source, minutes, range)
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn range_bounds(&self) -> Option<(String, String)> {
        if self.start_date > self.end_date
            || (self.start_date == self.end_date && self.start_hour > self.end_hour)
        {
            return None;
        }
        let start = self.start_date.and_hms_opt(self.start_hour, 0, 0)?;
        let end = self.end_date.and_hms_opt(self.end_hour, 59, 59)?;
        let start = Local.from_local_datetime(&start).earliest()?.to_rfc3339();
        let end = Local.from_local_datetime(&end).latest()?.to_rfc3339();
        Some((start, end))
    }

    fn range_window(&mut self, ctx: &egui::Context) {
        if !self.range_open {
            return;
        }
        let mut open = self.range_open;
        egui::Window::new("Choose log date and time range")
            .open(&mut open)
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if self.choosing_end {
                            "Choose end date"
                        } else {
                            "Choose start date"
                        })
                        .size(13.0)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text("Close calendar")
                            .clicked()
                        {
                            self.range_open = false;
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button("‹").clicked() {
                        self.calendar_month = shift_month(self.calendar_month, -1);
                    }
                    ui.label(self.calendar_month.format("%B %Y").to_string());
                    if ui.button("›").clicked() {
                        self.calendar_month = shift_month(self.calendar_month, 1);
                    }
                });
                egui::Grid::new("log_calendar")
                    .num_columns(7)
                    .min_col_width(38.0)
                    .max_col_width(38.0)
                    .min_row_height(34.0)
                    .spacing([4.0, 4.0])
                    .show(ui, |ui| {
                        for day in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                            ui.add_sized(
                                [38.0, 34.0],
                                egui::Label::new(RichText::new(day).color(MUTED))
                                    .halign(egui::Align::Center),
                            );
                        }
                        ui.end_row();
                        let first = self.calendar_month;
                        let days = (shift_month(first, 1) - ChronoDuration::days(1)).day();
                        let offset = first.weekday().num_days_from_monday();
                        for cell in 0..42 {
                            if cell >= offset && cell < offset + days {
                                let date = first + ChronoDuration::days((cell - offset) as i64);
                                let selected = date == self.start_date || date == self.end_date;
                                let between = date > self.start_date && date < self.end_date;
                                let label =
                                    RichText::new(date.day().to_string()).color(if selected {
                                        CYAN
                                    } else if between {
                                        TEXT
                                    } else {
                                        MUTED
                                    });
                                if ui
                                    .add_sized(
                                        [38.0, 34.0],
                                        egui::Button::new(label).selected(selected || between),
                                    )
                                    .clicked()
                                {
                                    if !self.choosing_end {
                                        self.start_date = date;
                                        self.end_date = date;
                                        self.choosing_end = true;
                                    } else {
                                        if date < self.start_date {
                                            self.end_date = self.start_date;
                                            self.start_date = date;
                                        } else {
                                            self.end_date = date;
                                        }
                                        self.choosing_end = false;
                                    }
                                }
                            } else {
                                ui.add_sized(
                                    [38.0, 34.0],
                                    egui::Button::new(" ")
                                        .frame(false)
                                        .sense(egui::Sense::hover()),
                                );
                            }
                            if cell % 7 == 6 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(format!("Start · {}", self.start_date.format("%b %d, %Y")));
                    egui::ComboBox::from_id_salt("log_start_hour")
                        .selected_text(format!("{:02}:00", self.start_hour))
                        .show_ui(ui, |ui| {
                            for hour in 0..24 {
                                ui.selectable_value(
                                    &mut self.start_hour,
                                    hour,
                                    format!("{hour:02}:00"),
                                );
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(format!("End · {}", self.end_date.format("%b %d, %Y")));
                    egui::ComboBox::from_id_salt("log_end_hour")
                        .selected_text(format!("{:02}:59", self.end_hour))
                        .show_ui(ui, |ui| {
                            for hour in 0..24 {
                                ui.selectable_value(
                                    &mut self.end_hour,
                                    hour,
                                    format!("{hour:02}:59"),
                                );
                            }
                        });
                });
                if let Some(error) = &self.range_error {
                    ui.colored_label(AMBER, error);
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.range_open = false;
                    }
                    if ui.button("Apply range").clicked() {
                        if self.range_bounds().is_some() {
                            self.custom_range = true;
                            self.range_error = None;
                            self.range_open = false;
                            self.refresh(ctx);
                        } else {
                            self.range_error = Some("End time must be after start time.".into());
                        }
                    }
                });
            });
        self.range_open = open && self.range_open;
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.access_pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.access_pending = None;
                    self.access_notice = Some(match result {
                        Ok(()) => "An administrator log window was opened. This window keeps its existing permissions.".into(),
                        Err(message) => message,
                    });
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.access_pending = None;
                    self.access_notice =
                        Some("The access request could not complete. Try again.".into());
                }
                _ => {}
            }
        }
        if !self.opened {
            self.opened = true;
            self.refresh(ctx);
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok(s) => self.snapshot = Some(s),
                        Err(e) => self.error = Some(e),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some("Log worker stopped. Refresh to retry.".into());
                }
                _ => {}
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R)) {
            self.refresh(ctx);
        }
        egui::TopBottomPanel::top("log_controls")
            .frame(panel(BG, 22))
            .show(ctx, |ui| {
                ui.heading("System logs");
                caption(
                    ui,
                    "Read recent system events and inspect their full messages.",
                );
                ui.add_space(10.0);
                let mut changed = false;
                ui.add_enabled_ui(self.pending.is_none(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("log_source")
                            .selected_text(logs::sources()[self.source])
                            .show_ui(ui, |ui| {
                                for (i, s) in logs::sources().iter().enumerate() {
                                    changed |=
                                        ui.selectable_value(&mut self.source, i, *s).changed();
                                }
                            });
                        egui::ComboBox::from_id_salt("log_time")
                            .selected_text(if self.custom_range {
                                "Quick range".into()
                            } else {
                                format!("Last {} minutes", self.minutes)
                            })
                            .show_ui(ui, |ui| {
                                for (n, s) in
                                    [(15, "15 minutes"), (60, "1 hour"), (1440, "24 hours")]
                                {
                                    if ui
                                        .selectable_label(
                                            !self.custom_range && self.minutes == n,
                                            s,
                                        )
                                        .clicked()
                                    {
                                        self.minutes = n;
                                        self.custom_range = false;
                                        changed = true;
                                    }
                                }
                            });
                        let range_label = if self.custom_range {
                            format!(
                                "{}–{}",
                                self.start_date.format("%b %d"),
                                self.end_date.format("%b %d")
                            )
                        } else {
                            "Calendar range…".to_string()
                        };
                        if ui.button(range_label).clicked() {
                            self.range_open = true;
                            self.choosing_end = false;
                            self.calendar_month = self.start_date.with_day(1).unwrap();
                        }
                        if self.custom_range && ui.button("Clear range").clicked() {
                            self.custom_range = false;
                            self.range_open = false;
                            self.range_error = None;
                            changed = true;
                        }
                        if ui.button("Refresh").clicked() {
                            self.refresh(ctx);
                        }
                    })
                });
                if changed {
                    self.refresh(ctx);
                }
                ui.horizontal(|ui| {
                    let search_width = (ui.available_width() - 150.0).max(160.0);
                    let search = ui.add_sized(
                        [search_width, ui.spacing().interact_size.y],
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search message, source, event ID…")
                            .desired_width(search_width),
                    );
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                        search.request_focus();
                    }
                    egui::ComboBox::from_id_salt("log_level")
                        .selected_text(&self.level)
                        .show_ui(ui, |ui| {
                            for level in [
                                "All", "Critical", "Error", "Warning", "Info", "Debug", "Other",
                            ] {
                                ui.selectable_value(&mut self.level, level.into(), level);
                            }
                        });
                });
                if self.pending.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading logs…");
                    });
                }
                if let Some(e) = &self.error {
                    ui.colored_label(AMBER, &e.message);
                }
                let restricted = self.error.as_ref().is_some_and(|e| e.permission_denied)
                    || self.snapshot.as_ref().is_some_and(|s| s.access_limited);
                if restricted {
                    if self.administrator_window {
                        ui.label("The operating system still restricts this source, even with administrator access.");
                    } else {
                        ui.label(if cfg!(windows) {
                            "Grant access to this source? The system will ask permission to open a separate administrator app window."
                        } else {
                            "Grant access to the remaining logs? The system will ask you to authenticate for this log query."
                        });
                        if ui.add_enabled(self.pending.is_none() && self.access_pending.is_none(),
                            egui::Button::new(if cfg!(windows) { "Open administrator window…" } else { "Grant log access…" })
                        ).clicked() { self.request_access(ctx); }
                    }
                    if ui.add_enabled(self.pending.is_none(), egui::Button::new("Show accessible logs")).clicked() {
                        self.source = logs::default_source();
                        self.elevated_reads = false;
                        self.refresh(ctx);
                    }
                }
                if self.administrator_window { caption(ui, "Administrator window"); }
                if self.access_pending.is_some() { ui.label("Waiting for your operating system's permission prompt…"); }
                if let Some(message) = &self.access_notice { ui.label(message); }
            });
        self.range_window(ctx);
        if let Some(e) = self
            .selected
            .and_then(|i| self.snapshot.as_ref()?.entries.get(i))
        {
            egui::SidePanel::right("log_details")
                .default_width(360.0)
                .width_range(260.0..=500.0)
                .frame(panel(PANEL, 18))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Event details");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("×")
                                .on_hover_text("Close event details")
                                .clicked()
                            {
                                self.selected = None;
                            }
                        });
                    });
                    if ui.button("Copy event").clicked() {
                        ctx.copy_text(e.details().to_owned());
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("system_log_details")
                        .show(ui, |ui| {
                            field(ui, "Time", &e.time);
                            field(ui, "Severity", &e.level);
                            field(ui, "Source / event", &e.source);
                            field(ui, "Message", &e.message);
                            ui.collapsing("Raw event", |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(e.details()).monospace())
                                        .wrap()
                                        .selectable(true),
                                );
                            });
                        });
                });
        }
        egui::CentralPanel::default()
            .frame(panel(BG, 22))
            .show(ctx, |ui| {
                let Some(snapshot) = &self.snapshot else {
                    if self.pending.is_none() && ui.button("Open system logs").clicked() {
                        self.refresh(ctx);
                    }
                    return;
                };
                let terms: Vec<_> = self
                    .query
                    .to_lowercase()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                let rows: Vec<_> = snapshot
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        (self.level == "All" || e.level == self.level)
                            && terms.iter().all(|t| e.search.contains(t))
                    })
                    .map(|(i, _)| i)
                    .collect();
                // Keep the list's top edge fixed when opening the details pane narrows it.
                ui.add(
                    egui::Label::new(RichText::new(&snapshot.note).size(11.0).color(MUTED))
                        .truncate(),
                )
                .on_hover_text(&snapshot.note);
                ui.label(format!(
                    "{} matching / {} loaded",
                    rows.len(),
                    snapshot.entries.len()
                ));
                if rows.is_empty() {
                    ui.label(
                        "No matching events. Change the filters or choose a longer time range.",
                    );
                }
                let clicked = self.list.show(ui, &snapshot.entries, &rows, self.selected);
                if let Some(i) = clicked {
                    self.selected = Some(i);
                    ctx.request_repaint();
                }
            });
    }
}

fn shift_month(date: NaiveDate, delta: i32) -> NaiveDate {
    let first = date.with_day(1).unwrap();
    let month_index = first.year() * 12 + first.month0() as i32 + delta;
    let year = month_index.div_euclid(12);
    let month = month_index.rem_euclid(12) as u32 + 1;
    NaiveDate::from_ymd_opt(year, month, 1).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, app: &mut Logs, events: Vec<egui::Event>) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    }

    fn click(ctx: &egui::Context, app: &mut Logs, index: usize) -> egui::FullOutput {
        let pos = app
            .list
            .visible_rows
            .iter()
            .find(|(i, _)| *i == index)
            .unwrap()
            .1
            .center();
        frame(ctx, app, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        frame(ctx, app, vec![])
    }

    fn selected_containers(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|s| {
                matches!(&s.shape,
                    egui::Shape::Rect(r) if r.fill == egui::Color32::from_rgb(17, 39, 48)
                )
            })
            .count()
    }

    #[test]
    fn selecting_and_scrolling_keeps_one_highlight_and_stable_row_positions() {
        let ctx = egui::Context::default();
        theme(&ctx);
        let mut app = Logs::new();
        app.opened = true;
        app.snapshot = Some(logs::Snapshot {
            note: "Recent events. Search filters loaded events. Visibility depends on OS permissions; private fields may be redacted.".into(),
            access_limited: false,
            entries: (0..50).map(|i| logs::Entry::test_entry(
                "2026-10-26T14:00:00+02:00".into(),
                "Info".into(), format!("Long service name / {i}"),
                "A long log message that should remain contained when the inspector opens and reduces the width of the list.".into(),
                format!("Event {i}"), format!("event {i}"),
            )).collect(),
        });
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let original_top = app.list.visible_rows[0].1.top();
        for index in [2, 5, 1, 7, 3] {
            let output = click(&ctx, &mut app, index);
            assert_eq!(app.selected, Some(index));
            assert_eq!(
                selected_containers(&output),
                1,
                "only the chosen event should have selection styling"
            );
            assert!(
                (app.list.visible_rows[0].1.top() - original_top).abs() < 0.1,
                "opening details moved the list"
            );
        }
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(egui::pos2(200.0, 400.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -500.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..30 {
            frame(&ctx, &mut app, vec![]);
        }
        let index = app
            .list
            .visible_rows
            .iter()
            .find(|(_, r)| r.top() > original_top + 10.0 && r.bottom() < 650.0)
            .unwrap()
            .0;
        assert!(index > 3, "the fixture should have scrolled");
        let scroll_before = app.list.scroll_offset();
        let output = click(&ctx, &mut app, index);
        assert_eq!(app.selected, Some(index));
        assert_eq!(selected_containers(&output), 1);
        assert_eq!(
            app.list.scroll_offset(),
            scroll_before,
            "selection changed the scroll offset"
        );

        // Exercise both viewport edges repeatedly; these clicks used to move the list.
        for step in 0..20 {
            let fully_visible: Vec<_> = app
                .list
                .visible_rows
                .iter()
                .filter(|(_, r)| r.top() >= original_top && r.bottom() < 700.0)
                .copied()
                .collect();
            let (index, before) = if step % 2 == 0 {
                *fully_visible.first().unwrap()
            } else {
                *fully_visible.last().unwrap()
            };
            let offset = app.list.scroll_offset();
            let output = click(&ctx, &mut app, index);
            assert_eq!(app.selected, Some(index));
            assert_eq!(selected_containers(&output), 1);
            assert_eq!(app.list.scroll_offset(), offset);
            let after = app
                .list
                .visible_rows
                .iter()
                .find(|(i, _)| *i == index)
                .unwrap()
                .1;
            assert_eq!(before.top(), after.top(), "clicked entry moved");
        }

        // Changing the filter must keep the entry identity, even when its row position changes.
        app.query = format!("event {}", app.selected.unwrap());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let index = app.list.visible_rows[0].0;
        click(&ctx, &mut app, index);
        assert_eq!(app.selected, Some(index));
    }
}
