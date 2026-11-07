use crate::ui::*;
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use std::{
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Images,
    Volumes,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Images => "images",
            Self::Volumes => "volumes",
        }
    }

    fn fields(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Images => &[
                ("Repository", "Repository"),
                ("Tag", "Tag"),
                ("ID", "Image ID"),
                ("CreatedAt", "Created"),
                ("Size", "Size"),
            ],
            Self::Volumes => &[
                ("Name", "Name"),
                ("Driver", "Driver"),
                ("Scope", "Scope"),
                ("Mountpoint", "Mount point"),
            ],
        }
    }

    fn args(self) -> Vec<String> {
        let mut args = match self {
            Self::Images => vec!["image", "ls", "--all", "--no-trunc"],
            Self::Volumes => vec!["volume", "ls"],
        };
        args.extend(["--format", "{{json .}}"]);
        args.into_iter().map(str::to_owned).collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Resource {
    cells: Vec<String>,
    search: String,
}

fn parse(kind: Kind, text: &str) -> Result<Vec<Resource>, String> {
    let mut rows: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line)
                .map_err(|error| format!("Invalid Docker {} response: {error}", kind.label()))?;
            let cells: Result<Vec<_>, _> = kind
                .fields()
                .iter()
                .map(|(key, _)| match value[*key].as_str() {
                    Some(text) => Ok(if text.is_empty() {
                        "—".into()
                    } else {
                        text.to_owned()
                    }),
                    None => Err(format!("Docker {} response is missing {key}", kind.label())),
                })
                .collect();
            let cells = cells?;
            let search = cells.join(" ").to_lowercase();
            Ok(Resource { cells, search })
        })
        .collect::<Result<_, String>>()?;
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    Ok(rows)
}

pub struct Resources {
    kind: Kind,
    rows: Vec<Resource>,
    rx: Option<Receiver<Result<Vec<Resource>, String>>>,
    loaded: bool,
    last_scan: Instant,
    query: String,
    auto: bool,
    error: Option<String>,
}

impl Resources {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            rows: Vec::new(),
            rx: None,
            loaded: false,
            last_scan: Instant::now(),
            query: String::new(),
            auto: true,
            error: None,
        }
    }

    pub fn status(&self) -> &'static str {
        if self.error.is_some() {
            "Docker unavailable / refresh to retry"
        } else {
            match self.kind {
                Kind::Images => "Docker images",
                Kind::Volumes => "Docker volumes",
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture(kind: Kind, text: &str) -> Self {
        let mut list = Self::new(kind);
        list.rows = parse(kind, text).unwrap();
        list.loaded = true;
        list.auto = false;
        list
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let kind = self.kind;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result =
                crate::docker::client::docker(&kind.args()).and_then(|text| parse(kind, &text));
            tx.send(result).ok();
            ctx.request_repaint();
        });
    }

    pub fn show(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if let Some(result) = self.rx.as_ref().and_then(crate::worker::poll) {
            self.rx = None;
            self.loaded = true;
            self.last_scan = Instant::now();
            match result {
                Ok(rows) => {
                    self.rows = rows;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if !self.loaded || self.auto && self.last_scan.elapsed() >= Duration::from_secs(3) {
            self.refresh(ctx);
        }
        if self.auto {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        ui.horizontal_wrapped(|ui| {
            let width = (ui.available_width() - 180.0).clamp(180.0, 480.0);
            egui::Frame::new()
                .fill(PANEL)
                .stroke(egui::Stroke::new(1.0_f32, LINE))
                .corner_radius(5)
                .inner_margin(egui::Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.add_sized(
                        [width, 22.0],
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(format!("Search {}…", self.kind.label()))
                            .frame(false),
                    );
                });
            ui.checkbox(&mut self.auto, "Auto refresh");
        });
        ui.add_space(8.0);
        caption(
            ui,
            &format!(
                "{} {}",
                self.rows.len(),
                if self.kind == Kind::Images {
                    "image entries"
                } else {
                    "volumes"
                }
            ),
        );
        if let Some(error) = &self.error {
            ui.colored_label(RED, error);
        }
        ui.add_space(8.0);
        rule(ui);
        let query = self.query.to_lowercase();
        let terms: Vec<_> = query.split_whitespace().collect();
        let rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| terms.iter().all(|word| row.search.contains(word)))
            .collect();
        if rows.is_empty() {
            ui.add_space(18.0);
            let message = if !self.loaded {
                format!("Loading {}…", self.kind.label())
            } else if self.error.is_some() {
                "Ensure Docker is running and your account can access the daemon. Refresh to retry."
                    .into()
            } else if self.rows.is_empty() {
                format!("No {} in the current Docker context.", self.kind.label())
            } else {
                format!("No {} match your search.", self.kind.label())
            };
            ui.label(RichText::new(message).color(MUTED));
        }
        egui::ScrollArea::horizontal()
            .id_salt(("docker_resources_horizontal", self.kind.label()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width().max(760.0));
                let mut table = TableBuilder::new(ui)
                    .id_salt(("docker_resources", self.kind.label()))
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .striped(true)
                    .column(Column::remainder().at_least(180.0));
                for _ in 1..self.kind.fields().len() {
                    table = table.column(Column::initial(150.0).resizable(true));
                }
                table
                    .header(32.0, |mut header| {
                        for (_, title) in self.kind.fields() {
                            header.col(|ui| {
                                caption(ui, title);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(48.0, rows.len(), |mut row| {
                            for (index, cell) in rows[row.index()].cells.iter().enumerate() {
                                row.col(|ui| {
                                    let text = RichText::new(cell).color(if index == 0 {
                                        TEXT
                                    } else {
                                        MUTED
                                    });
                                    ui.add(egui::Label::new(text).truncate())
                                        .on_hover_text(cell);
                                });
                            }
                        });
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_images_and_preserves_multiple_tags_for_one_id() {
        let text = r#"{"Repository":"nginx","Tag":"latest","ID":"sha256:abc","CreatedAt":"2026-01-01","Size":"120MB"}
{"Repository":"nginx","Tag":"stable","ID":"sha256:abc","CreatedAt":"2026-01-01","Size":"120MB"}
{"Repository":"<none>","Tag":"<none>","ID":"sha256:def","CreatedAt":"2026-01-02","Size":"10MB"}"#;
        let rows = parse(Kind::Images, text).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].cells[0], "<none>");
        assert_eq!(rows[1].cells[1], "latest");
        assert!(rows[2].search.contains("nginx stable sha256:abc"));
        assert_eq!(
            &Kind::Images.args()[..4],
            ["image", "ls", "--all", "--no-trunc"]
        );
    }

    #[test]
    fn parses_volumes_and_rejects_incomplete_or_malformed_inventory() {
        let text = r#"{"Name":"db-data","Driver":"local","Scope":"local","Mountpoint":"/var/lib/docker/volumes/db-data/_data"}"#;
        let rows = parse(Kind::Volumes, text).unwrap();
        assert_eq!(rows[0].cells[0], "db-data");
        assert!(rows[0].search.contains("db-data local"));
        for kind in [Kind::Images, Kind::Volumes] {
            assert!(parse(kind, "not json").is_err());
            assert!(parse(kind, "{}").is_err());
            assert!(parse(kind, "").unwrap().is_empty());
        }
    }
}
