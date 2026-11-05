use crate::ui::*;
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use std::{collections::{BTreeMap, HashSet}, process::{Command, Stdio}, sync::mpsc::{self, Receiver}, time::{Duration, Instant}};

const FORMAT: &str = r#"{"id":{{json .ID}},"name":{{json .Names}},"image":{{json .Image}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"project":{{json (.Label "com.docker.compose.project")}},"service":{{json (.Label "com.docker.compose.service")}}}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Container {
    id: String, name: String, image: String, state: String, status: String,
    ports: String, project: String, service: String,
}
impl Container {
    fn active(&self) -> bool { matches!(self.state.as_str(), "running" | "restarting" | "paused") }
    fn startable(&self) -> bool { matches!(self.state.as_str(), "created" | "exited") }
    fn matches(&self, query: &str) -> bool {
        let text = format!("{} {} {} {} {} {} {}", self.id, self.name, self.image, self.status, self.ports, self.project, self.service).to_lowercase();
        query.split_whitespace().all(|word| text.contains(&word.to_lowercase()))
    }
}
fn parse(text: &str) -> Result<Vec<Container>, String> {
    text.lines().filter(|line| !line.trim().is_empty()).map(|line| {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("Invalid Docker response: {e}"))?;
        let get = |key: &str| value[key].as_str().map(str::to_owned).ok_or_else(|| format!("Docker response is missing {key}"));
        let id = get("id")?;
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("Docker returned an invalid container ID".into()); }
        Ok(Container { id, name: get("name")?, image: get("image")?, state: get("state")?, status: get("status")?, ports: get("ports")?, project: get("project")?, service: get("service")? })
    }).collect()
}

// Drain both pipes while waiting so large inventories cannot fill an OS pipe.
pub(crate) fn docker(args: &[String]) -> Result<String, String> {
    use std::io::Read;
    let mut command = Command::new("docker");
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let mut child = command.spawn().map_err(|e| format!("Cannot run Docker: {e}. Install Docker and ensure it is on PATH."))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || { let mut b = Vec::new(); stdout.read_to_end(&mut b).map(|_| b) });
    let err = std::thread::spawn(move || { let mut b = Vec::new(); stderr.read_to_end(&mut b).map(|_| b) });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            result => { child.kill().ok(); child.wait().ok(); break Err(match result { Err(e) => e.to_string(), _ => "Docker timed out after 60 seconds. Refresh to check the result.".into() }); }
        }
    };
    let output = out.join().map_err(|_| "Docker output reader failed")?.map_err(|e| e.to_string())?;
    let error = err.join().map_err(|_| "Docker error reader failed")?.map_err(|e| e.to_string())?;
    if !status?.success() { return Err(String::from_utf8_lossy(&error).trim().to_owned()); }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

#[derive(Clone, Copy, PartialEq)]
enum Action { Start, Stop, Delete }
impl Action {
    fn label(self) -> &'static str { match self { Self::Start => "Start", Self::Stop => "Stop", Self::Delete => "Delete" } }
}
#[derive(Clone)]
struct Request { action: Action, name: String, containers: Vec<Container> }
struct ResultEvent { snapshot: Result<Vec<Container>, String>, notice: Option<Result<String, String>> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab { Containers, Images, Volumes }

pub struct Docker {
    tab: Tab,
    images: crate::docker_resources::Resources,
    volumes: crate::docker_resources::Resources,
    containers: Vec<Container>, rx: Option<Receiver<ResultEvent>>, scanned: bool,
    last_scan: Instant, query: String, running_only: bool, auto: bool,
    terminal_rx: Option<Receiver<Result<String, String>>>,
    mutating: bool, pending: Option<Request>, expanded: HashSet<String>, logs: Option<crate::docker_logs::ContainerLogs>,
    error: Option<String>, notice: Option<(String, bool)>, confirmation: Option<Request>,
}
impl Docker {
    pub fn new() -> Self { Self { tab: Tab::Containers, images: crate::docker_resources::Resources::new(crate::docker_resources::Kind::Images), volumes: crate::docker_resources::Resources::new(crate::docker_resources::Kind::Volumes), terminal_rx: None, mutating: false, pending: None, expanded: HashSet::new(), logs: None, containers: Vec::new(), rx: None, scanned: false, last_scan: Instant::now(), query: String::new(), running_only: false, auto: true, error: None, notice: None, confirmation: None } }
    pub fn status(&self) -> &str {
        if self.mutating { return "Applying Docker action…"; }
        match self.tab {
            Tab::Images => self.images.status(),
            Tab::Volumes => self.volumes.status(),
            Tab::Containers if self.error.is_some() => "Docker unavailable / refresh to retry",
            Tab::Containers => "Docker containers",
        }
    }
    fn launch(&mut self, ctx: &egui::Context, request: Option<Request>) {
        if self.rx.is_some() {
            if request.is_some() { self.pending = request; self.mutating = true; }
            return;
        }
        self.mutating = request.is_some();
        let (tx, rx) = mpsc::channel(); self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let notice = request.map(|r| {
                let mut args = vec![match r.action { Action::Start => "start", Action::Stop => "stop", Action::Delete => "rm" }.to_owned()];
                if r.action == Action::Delete { args.push("--force".into()); }
                args.extend(r.containers.iter().map(|c| c.id.clone()));
                docker(&args).map(|_| format!("{} completed for {}. Refreshed container status.", r.action.label(), r.name))
            });
            let snapshot = docker(&["ps".into(), "--all".into(), "--no-trunc".into(), "--format".into(), FORMAT.into()]).and_then(|s| parse(&s));
            tx.send(ResultEvent { snapshot, notice }).ok(); ctx.request_repaint();
        });
    }
    pub fn poll(&mut self) {
        if let Some(rx) = &self.terminal_rx && let Ok(result) = rx.try_recv() {
            self.terminal_rx = None;
            self.notice = Some(match result { Ok(text) => (text, false), Err(text) => (text, true) });
        }
        if let Some(rx) = &self.rx && let Ok(event) = rx.try_recv() {
            self.rx = None; self.mutating = self.pending.is_some(); self.scanned = true; self.last_scan = Instant::now();
            match event.snapshot { Ok(containers) => { if self.containers != containers { self.containers = containers; } self.error = None; }, Err(e) => self.error = Some(e) }
            if let Some(notice) = event.notice { self.notice = Some(match notice { Ok(s) => (s, false), Err(e) => (format!("Docker action failed (some containers may have changed): {e}"), true) }); }
        }
    }
    pub fn close_logs(&mut self) { self.logs = None; }
    pub fn show(&mut self, ctx: &egui::Context) {
        self.poll();
        if self.rx.is_none() && let Some(request) = self.pending.take() { self.launch(ctx, Some(request)); }
        if self.tab == Tab::Containers && (!self.scanned || (self.auto && self.confirmation.is_none() && self.last_scan.elapsed() >= Duration::from_secs(3))) { self.launch(ctx, None); }
        let close = self.logs.as_mut().is_some_and(|logs| logs.show(ctx));
        if close { self.close_logs(); }
        let mut request = None;
        let mut open_logs = None;
        let mut open_terminal = None;
        egui::CentralPanel::default().frame(panel(BG, 28)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Docker");
                    ui.label(RichText::new("Containers, images & volumes").color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Background scans keep the controls, colors and layout unchanged.
                    if ui.add_enabled(!self.mutating, egui::Button::new("Refresh")).clicked() {
                        match self.tab {
                            Tab::Containers => self.launch(ctx, None),
                            Tab::Images => self.images.refresh(ctx),
                            Tab::Volumes => self.volumes.refresh(ctx),
                        }
                    }
                });
            });
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                for (tab, label) in [(Tab::Containers, "Containers"), (Tab::Images, "Images"), (Tab::Volumes, "Volumes")] {
                    if ui.selectable_label(self.tab == tab, label).clicked() {
                        self.tab = tab;
                        if tab != Tab::Containers { self.close_logs(); }
                    }
                }
            });
            ui.add_space(12.0);
            match self.tab {
                Tab::Images => { self.images.show(ctx, ui); return; }
                Tab::Volumes => { self.volumes.show(ctx, ui); return; }
                Tab::Containers => {}
            }
            ui.horizontal_wrapped(|ui| {
                let width = (ui.available_width() - 300.0).clamp(180.0, 480.0);
                egui::Frame::new().fill(PANEL).stroke(egui::Stroke::new(1.0, LINE)).corner_radius(5)
                    .inner_margin(egui::Margin::symmetric(12, 9)).show(ui, |ui| {
                        ui.add_sized([width, 22.0], egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search containers, images or ports…").frame(false));
                    });
                ui.checkbox(&mut self.running_only, "Running only");
                ui.checkbox(&mut self.auto, "Auto refresh");
            });
            ui.add_space(8.0);
            caption(ui, &format!("{} containers · {} running", self.containers.len(), self.containers.iter().filter(|c| c.state == "running").count()));
            if let Some(error) = &self.error { ui.colored_label(RED, error); }
            if let Some((notice, error)) = &self.notice { ui.colored_label(if *error { RED } else { CYAN }, notice); }
            ui.add_space(8.0);
            rule(ui);
            let rows = display_rows(&self.containers, &self.expanded, &self.query, self.running_only);
            let busy = self.mutating || self.error.is_some() || self.confirmation.is_some();
            if rows.is_empty() {
                ui.add_space(18.0);
                ui.label(RichText::new(if !self.scanned { "Loading containers…" } else if self.error.is_some() { "Ensure Docker is running and your account can access the daemon. Refresh to retry." } else if self.containers.is_empty() { "No containers in the current Docker context." } else { "No containers match your filters." }).color(MUTED));
            }
            egui::ScrollArea::horizontal().id_salt("docker_table_horizontal").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_min_width(ui.available_width().max(860.0));
                TableBuilder::new(ui).id_salt("docker_table")
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center)).striped(true)
                    .column(Column::remainder().at_least(180.0))
                    .column(Column::initial(150.0).resizable(true))
                    .column(Column::initial(110.0).resizable(true))
                    .column(Column::initial(140.0).resizable(true))
                    .column(Column::exact(205.0))
                    .header(32.0, |mut header| {
                        for name in ["Name", "Image / service", "Status", "Ports", "Actions"] {
                            header.col(|ui| { caption(ui, name); });
                        }
                    }).body(|body| {
                        body.rows(48.0, rows.len(), |mut row| {
                            let entry = &rows[row.index()];
                            row.col(|ui| {
                                ui.push_id(&entry.key, |ui| {
                                    if entry.project {
                                        let expanded = self.expanded.contains(&entry.name);
                                        if compose_toggle(ui, expanded).clicked()
                                            && !self.expanded.remove(&entry.name) { self.expanded.insert(entry.name.clone()); }
                                    } else { ui.add_space(if entry.child { 34.0 } else { 8.0 }); }
                                    ui.add(egui::Label::new(RichText::new(&entry.name).strong().color(if entry.project { CYAN } else { TEXT })).truncate())
                                        .on_hover_text(&entry.name);
                                });
                            });
                            row.col(|ui| {
                                if entry.project { badge(ui, "COMPOSE", CYAN); }
                                else {
                                    let c = &entry.containers[0];
                                    ui.add(egui::Label::new(RichText::new(&c.image).size(12.0)).truncate()).on_hover_text(&c.image);
                                }
                            });
                            row.col(|ui| {
                                if entry.project { ui.label(RichText::new(format!("{} / {} running", entry.running, entry.containers.len())).size(12.0).color(MUTED)); }
                                else {
                                    let c = &entry.containers[0];
                                    badge(ui, &c.state, if c.state == "running" { CYAN } else { MUTED });
                                    ui.response().on_hover_text(&c.status);
                                }
                            });
                            row.col(|ui| {
                                if !entry.project {
                                    let c = &entry.containers[0];
                                    ui.add(egui::Label::new(RichText::new(if c.ports.is_empty() { "—" } else { &c.ports }).monospace().size(11.0).color(MUTED)).truncate())
                                        .on_hover_text(format!("{}\nService: {}\nID: {}", c.ports, c.service, c.id));
                                }
                            });
                            row.col(|ui| {
                                ui.push_id((&entry.key, "actions"), |ui| {
                                    ui.spacing_mut().item_spacing.x = 5.0;
                                    ui.spacing_mut().button_padding = egui::vec2(6.0, 5.0);
                                    actions(ui, &entry.name, &entry.containers, busy, &mut request);
                                    if !entry.project {
                                        let container = &entry.containers[0];
                                        if action_icon(ui, ">_", "Open container terminal", CYAN, !busy && self.terminal_rx.is_none() && container.state == "running").clicked() {
                                            open_terminal = Some(container.id.clone());
                                        }
                                        if ui.button(RichText::new("Logs").color(CYAN)).clicked() { open_logs = Some(container.clone()); }
                                    }
                                });
                            });
                        });
                    });
            });
        });
        if let Some(id) = open_terminal {
            let (tx, rx) = mpsc::channel();
            self.terminal_rx = Some(rx);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                tx.send(crate::docker_terminal::open(&id)).ok();
                ctx.request_repaint();
            });
        }
        if let Some(c) = open_logs {
            self.logs = Some(crate::docker_logs::ContainerLogs::new(c.id, c.name, ctx.clone()));
        }
        if let Some(request) = request {
            if request.action == Action::Delete { self.confirmation = Some(request); }
            else { self.launch(ctx, Some(request)); }
        }
        if let Some(request) = self.confirmation.clone() {
            egui::Modal::new(egui::Id::new("docker-delete")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading(format!("Delete {}?", request.name));
                ui.label(format!("Remove {} container(s), stopping running containers. Their writable data will be lost. Images, volumes and networks are kept.", request.containers.len()));
                for c in &request.containers { ui.label(&c.name); }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { self.confirmation = None; }
                    if ui.button(RichText::new("Delete containers").color(RED)).clicked() { self.confirmation = None; self.launch(ctx, Some(request.clone())); }
                });
            });
        }
    }
}
// Use egui's built-in disclosure painter, which does not depend on font glyph coverage.
fn compose_toggle(ui: &mut egui::Ui, expanded: bool) -> egui::Response {
    let response = ui.add(egui::Button::new("").frame(false).min_size(egui::vec2(22.0, 28.0)))
        .on_hover_text(if expanded { "Collapse project" } else { "Expand project" });
    let mut icon_response = response.clone();
    icon_response.rect = egui::Rect::from_center_size(response.rect.center(), egui::vec2(12.0, 12.0));
    egui::collapsing_header::paint_default_icon(ui, if expanded { 1.0 } else { 0.0 }, &icon_response);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, if expanded { "Collapse project" } else { "Expand project" }));
    response
}

// These symbols ship in egui's default font set; no custom icon artwork.
fn action_icon(ui: &mut egui::Ui, icon: &str, label: &str, color: egui::Color32, enabled: bool) -> egui::Response {
    let text = RichText::new(icon).size(16.0).color(color);
    let text = if icon == ">_" { text.monospace() } else { text };
    let response = ui
        .add_enabled(enabled, egui::Button::new(text).min_size(egui::vec2(30.0, 30.0)))
        .on_hover_text(label)
        .on_disabled_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

fn actions(ui: &mut egui::Ui, name: &str, containers: &[Container], busy: bool, request: &mut Option<Request>) {
    for action in [Action::Start, Action::Stop, Action::Delete] {
        let targets: Vec<_> = containers.iter().filter(|c| match action { Action::Start => c.startable(), Action::Stop => c.active(), Action::Delete => true }).cloned().collect();
        let icon = match action { Action::Start => "▶", Action::Stop => "■", Action::Delete => "🗑" };
        if action_icon(ui, icon, action.label(), if action == Action::Delete { RED } else { CYAN }, !busy && !targets.is_empty()).clicked() {
            *request = Some(Request { action, name: name.into(), containers: targets });
        }
    }
}
struct DisplayRow {
    key: String,
    name: String,
    project: bool,
    child: bool,
    running: usize,
    containers: Vec<Container>,
}
fn display_rows(containers: &[Container], expanded: &HashSet<String>, query: &str, running_only: bool) -> Vec<DisplayRow> {
    let mut groups: BTreeMap<String, Vec<Container>> = BTreeMap::new();
    let mut standalone = Vec::new();
    for c in containers {
        if c.project.is_empty() { standalone.push(c.clone()); }
        else { groups.entry(c.project.clone()).or_default().push(c.clone()); }
    }
    let visible = |c: &Container| (!running_only || c.state == "running") && c.matches(query);
    let container_row = |c: &Container, child| DisplayRow {
        key: c.id.clone(), name: c.name.clone(), project: false, child,
        running: usize::from(c.state == "running"), containers: vec![c.clone()],
    };
    let mut rows = Vec::new();
    for (project, mut members) in groups {
        members.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        if !members.iter().any(&visible) { continue; }
        rows.push(DisplayRow {
            key: format!("compose:{project}"), name: project.clone(), project: true, child: false,
            running: members.iter().filter(|c| c.state == "running").count(), containers: members.clone(),
        });
        if expanded.contains(&project) {
            rows.extend(members.iter().filter(|c| visible(c)).map(|c| container_row(c, true)));
        }
    }
    standalone.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    rows.extend(standalone.iter().filter(|c| visible(c)).map(|c| container_row(c, false)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_compose_and_standalone_and_searches() {
        let row = |project: &str| serde_json::json!({"id": "a".repeat(64), "name":"web-1", "image":"nginx", "state":"running", "status":"Up 2 hours (healthy)", "ports":"0.0.0.0:8080->80/tcp", "project":project, "service":"web"}).to_string();
        let containers = parse(&format!("{}\n{}\n", row("shop"), row(""))).unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].project, "shop");
        assert!(containers[1].project.is_empty());
        assert!(containers[0].matches("SHOP 8080 healthy"));
        assert!(!containers[1].matches("shop"));
        assert!(containers[0].active());
        assert!(!containers[0].startable());
    }
    #[test]
    fn groups_are_stable_and_actions_keep_hidden_members() {
        let make = |id: char, name: &str, state: &str, project: &str| Container {
            id: id.to_string().repeat(64), name: name.into(), image: "image".into(), state: state.into(),
            status: state.into(), ports: String::new(), project: project.into(), service: String::new(),
        };
        let mut inventory = vec![make('a', "web", "running", "shop"), make('b', "db", "exited", "shop"), make('c', "cache", "running", "")];
        let expanded = HashSet::from(["shop".into()]);
        let rows = display_rows(&inventory, &expanded, "", false);
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["shop", "db", "web", "cache"]);
        assert!(rows[1].child);
        let filtered = display_rows(&inventory, &expanded, "web", true);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].containers.len(), 2, "group actions must include filtered children");
        inventory.reverse();
        assert_eq!(display_rows(&inventory, &expanded, "", false).iter().map(|r| &r.key).collect::<Vec<_>>(), rows.iter().map(|r| &r.key).collect::<Vec<_>>());
    }
    #[test]
    fn layout_renders_at_minimum_and_full_width() {
        for width in [760.0, 1200.0] {
            let ctx = egui::Context::default();
            let mut docker = Docker::new();
            docker.scanned = true;
            docker.auto = false;
            docker.containers.push(Container { id: "a".repeat(64), name: "web".into(), image: "nginx".into(), state: "running".into(), status: "Up 2 hours".into(), ports: "8080->80/tcp".into(), project: "shop".into(), service: "web".into() });
            docker.expanded.insert("shop".into());
            for _ in 0..3 {
                let output = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 450.0))), ..Default::default() }, |ctx| docker.show(ctx));
                assert!(!output.shapes.is_empty());
            }
        }
    }
    #[test]
    fn background_refresh_does_not_change_controls_or_layout() {
        let ctx = egui::Context::default();
        theme(&ctx);
        ctx.style_mut(|style| style.animation_time = 0.0);
        let mut docker = Docker::new();
        docker.scanned = true;
        docker.auto = false;
        let render = |docker: &mut Docker| {
            ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 580.0))), ..Default::default() }, |ctx| docker.show(ctx))
        };
        for _ in 0..3 { render(&mut docker); }
        let before = render(&mut docker);
        let (_sender, receiver) = mpsc::channel();
        docker.rx = Some(receiver);
        let during = render(&mut docker);
        let before = format!("{:?}", before.shapes);
        let during = format!("{:?}", during.shapes);
        if before != during {
            let index = before.bytes().zip(during.bytes()).position(|(a, b)| a != b).unwrap_or(0);
            panic!("refresh shape differs near byte {index}: before {:?}, during {:?}", &before[index.saturating_sub(40)..(index+100).min(before.len())], &during[index.saturating_sub(40)..(index+100).min(during.len())]);
        }
    }
    #[test]
    fn action_icons_exist_in_bundled_fonts_and_disclosure_is_clickable() {
        let ctx = egui::Context::default();
        theme(&ctx);
        let mut expanded = false;
        let mut rect = egui::Rect::NOTHING;
        let mut render = |events: Vec<egui::Event>| {
            ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                ctx.fonts_mut(|fonts| {
                    for icon in ['▶', '■', '🗑', '>', '_'] {
                        assert!(fonts.has_glyph(&egui::FontId::proportional(16.0), icon), "Missing action icon: {icon}");
                    }
                });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = compose_toggle(ui, expanded);
                    rect = response.rect;
                    if response.clicked() { expanded = !expanded; }
                });
            })
        };
        let output = render(Vec::new());
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Path(path) if path.points.len() == 3)), "Disclosure arrow must be painted independently of fonts");
        // Release the closure's borrows before using the rectangle.
        drop(render);
        let point = rect.center();
        for pressed in [true, false] {
            let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerMoved(point), egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }], ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if compose_toggle(ui, expanded).clicked() { expanded = !expanded; }
                });
            });
        }
        assert!(expanded);
    }
    #[test]
    fn tabs_switch_lists_and_keep_container_filters() {
        use crate::docker_resources::{Kind, Resources};
        let ctx = egui::Context::default();
        theme(&ctx);
        let mut docker = Docker::new();
        docker.scanned = true;
        docker.auto = false;
        docker.query = "web".into();
        docker.running_only = true;
        docker.expanded.insert("shop".into());
        docker.images = Resources::fixture(Kind::Images, r#"{"Repository":"postgres","Tag":"17","ID":"sha256:abc","CreatedAt":"2026-01-01","Size":"100MB"}"#);
        docker.volumes = Resources::fixture(Kind::Volumes, r#"{"Name":"db-data","Driver":"local","Scope":"local","Mountpoint":"/data"}"#);
        let frame = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 650.0))),
            events,
            ..Default::default()
        };
        for (label, tab, expected) in [("Images", Tab::Images, "postgres"), ("Volumes", Tab::Volumes, "db-data"), ("Containers", Tab::Containers, "Running only")] {
            let output = ctx.run(frame(vec![]), |ctx| docker.show(ctx));
            let pos = output.shapes.iter().find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape && text.galley.text() == label {
                    Some(text.pos + text.galley.size() * 0.5)
                } else { None }
            }).expect("tab label");
            for pressed in [true, false] {
                let _ = ctx.run(frame(vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
                }]), |ctx| docker.show(ctx));
            }
            assert_eq!(docker.tab, tab);
            let output = ctx.run(frame(vec![]), |ctx| docker.show(ctx));
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == expected)), "Missing {expected} in {label}");
            if tab != Tab::Containers {
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Running only")));
            }
        }
        assert_eq!(docker.query, "web");
        assert!(docker.running_only);
        assert!(docker.expanded.contains("shop"));
    }

    #[test]
    fn rejects_malformed_inventory() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"id":"--help"}"#).is_err());
        assert!(parse("").unwrap().is_empty());
    }
}
