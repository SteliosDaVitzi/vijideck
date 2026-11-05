use crate::ui::*;
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone)]
struct Process {
    pid: u32,
    parent: Option<u32>,
    same_user: Option<bool>,
    start: u64,
    name: String,
    exe: Option<PathBuf>,
    cpu: f32,
    memory: u64,
    status: String,
    icon: Option<egui::TextureHandle>,
}
enum ProcessRow<'a> {
    Section(&'static str),
    AppGroup {
        name: String,
        pid: u32,
        count: usize,
        icon: Option<egui::TextureHandle>,
        cpu: f32,
        memory: u64,
    },
    Process(&'a Process),
}
type ProcessGroup<'a> = (ProcessRow<'a>, Vec<&'a Process>);

enum Command {
    Scan,
    Kill(Process),
    Open(PathBuf),
}
enum Event {
    Rows(Vec<Process>),
    Notice(Result<String, String>),
}
pub struct Processes {
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Event>,
    rows: Vec<Process>,
    query: String,
    sort: usize,
    descending: bool,
    auto: bool,
    busy: bool,
    last: Instant,
    notice: Option<Result<String, String>>,
    confirmation: Option<Process>,
    expanded: std::collections::HashSet<u32>,
}
impl Processes {
    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        !self.rows.is_empty()
    }
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut icons = std::collections::HashMap::new();
            let mut icon_order = VecDeque::new();
            let mut system = System::new();
            while let Ok(command) = commands.recv() {
                let event = match command {
                    Command::Scan => {
                        let mut icons_loaded = 0;
                        system.refresh_processes_specifics(
                            ProcessesToUpdate::All,
                            true,
                            ProcessRefreshKind::nothing()
                                .with_cpu()
                                .with_memory()
                                .with_user(UpdateKind::OnlyIfNotSet)
                                .with_exe(UpdateKind::OnlyIfNotSet),
                        );
                        let current_user = system.process(Pid::from_u32(std::process::id())).and_then(|p| p.user_id());
                        Event::Rows(
                            system
                                .processes()
                                .iter()
                                .map(|(pid, p)| Process {
                                    icon: p.exe().and_then(|path| {
                                        let path = path.to_owned();
                                        if !icons.contains_key(&path) {
                                            if icons_loaded >= 128 {
                                                return None;
                                            }
                                            let icon = executable_icon(&path).map(|image| {
                                                ctx.load_texture(
                                                    path.to_string_lossy(),
                                                    image,
                                                    egui::TextureOptions::LINEAR,
                                                )
                                            });
                                            icons.insert(path.clone(), icon);
                                            icon_order.push_back(path.clone());
                                            icons_loaded += 1;
                                            if icon_order.len() > 128
                                                && let Some(expired) = icon_order.pop_front()
                                            {
                                                icons.remove(&expired);
                                            }
                                        }
                                        icons.get(&path).cloned().flatten()
                                    }),
                                    pid: pid.as_u32(),
                                    parent: p.parent().map(|parent| parent.as_u32()),
                                    same_user: current_user.zip(p.user_id()).map(|(current, owner)| current == owner),
                                    start: p.start_time(),
                                    name: p.name().to_string_lossy().into_owned(),
                                    exe: p.exe().map(ToOwned::to_owned),
                                    cpu: p.cpu_usage(),
                                    memory: p.memory(),
                                    status: format!("{:?}", p.status()),
                                })
                                .collect(),
                        )
                    }
                    Command::Kill(p) => Event::Notice(kill(&p)),
                    Command::Open(path) => Event::Notice(open_folder(&path)),
                };
                if events.send(event).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            tx,
            rx,
            rows: vec![],
            query: String::new(),
            sort: 0,
            descending: false,
            auto: true,
            busy: false,
            last: Instant::now(),
            notice: None,
            confirmation: None,
            expanded: std::collections::HashSet::new(),
        }
    }
    fn refresh(&mut self) {
        if !self.busy {
            self.busy = self.tx.send(Command::Scan).is_ok();
        }
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Rows(rows) => {
                    self.rows = rows;
                    self.busy = false;
                    self.last = Instant::now();
                }
                Event::Notice(result) => {
                    self.notice = Some(result);
                    self.refresh();
                }
            }
        }
        if self.confirmation.is_none()
            && (self.rows.is_empty() || self.auto && self.last.elapsed() >= Duration::from_secs(2))
        {
            self.refresh();
        }
        egui::CentralPanel::default().frame(panel(BG, 24)).show(ctx, |ui| {
            ui.heading("Processes");
            ui.label(RichText::new("Running applications and background processes").color(MUTED));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_sized([300.0, 34.0], egui::TextEdit::singleline(&mut self.query).hint_text("Search name, PID or executable path").vertical_align(egui::Align::Center));
                if ui.button("Refresh").clicked() { self.refresh(); }
                ui.checkbox(&mut self.auto, "Auto refresh");
                ui.label(format!("{} processes", self.rows.len()));
            });
            if let Some(result) = &self.notice {
                ui.label(RichText::new(match result { Ok(s) | Err(s) => s }).color(if result.is_ok() { CYAN } else { RED }));
            }
            ui.label(RichText::new("CPU may exceed 100% across cores. Protected processes may hide their executable path.").small().color(MUTED));
            let query = self.query.trim().to_lowercase();
            let rows: Vec<_> = self.rows.iter().collect();
            let ordered = sorted_process_rows(&rows, self.sort, self.descending, &self.expanded, &query);
            if ordered.iter().all(|row| matches!(row, ProcessRow::Section(_))) { ui.label(if self.busy { "Loading processes…" } else { "No matching processes." }); }
            TableBuilder::new(ui).id_salt("process_table").cell_layout(egui::Layout::left_to_right(egui::Align::Center)).striped(true).column(Column::remainder().at_least(130.0))
                .column(Column::initial(65.0)).column(Column::initial(65.0)).column(Column::initial(90.0))
                .column(Column::initial(90.0)).column(Column::initial(90.0)).header(30.0, |mut header| {
                    for (i, name) in ["Name", "PID", "CPU", "Memory", "Status", "Actions"].iter().enumerate() {
                        header.col(|ui| {
                            if i < 5 {
                                let label = if self.sort == i {
                                    format!("{name} {}", if self.descending { "▼" } else { "▲" })
                                } else { (*name).to_owned() };
                                if ui.selectable_label(self.sort == i, label).clicked() {
                                    if self.sort == i { self.descending = !self.descending; } else { self.sort = i; self.descending = matches!(i, 2 | 3); }
                                    ctx.request_repaint();
                                }
                            } else { ui.label(*name); }
                        });
                    }
                }).body(|body| {
                    body.rows(36.0, ordered.len(), |mut row| {
                        match &ordered[row.index()] {
                        ProcessRow::Section(section) => {
                            row.col(|ui| { ui.label(RichText::new(*section).strong().color(CYAN)); });
                            for _ in 0..5 { row.col(|_| {}); }
                        }
                        ProcessRow::AppGroup { name, pid, count, icon, cpu, memory } => {
                            let expanded = self.expanded.contains(pid);
                            row.col(|ui| {
                                let (r, response) = ui.allocate_exact_size(egui::vec2(14.0, 20.0), egui::Sense::click());
                                let c = r.center();
                                let points = if expanded { vec![c + egui::vec2(-3.5, -2.0), c + egui::vec2(3.5, -2.0), c + egui::vec2(0.0, 2.5)] } else { vec![c + egui::vec2(-2.0, -3.5), c + egui::vec2(-2.0, 3.5), c + egui::vec2(2.5, 0.0)] };
                                ui.painter().add(egui::Shape::convex_polygon(points, if response.hovered() { CYAN } else { MUTED }, egui::Stroke::NONE));
                                if response.clicked() && !self.expanded.remove(pid) { self.expanded.insert(*pid); }
                                if let Some(icon) = icon { ui.add(egui::Image::new(icon).fit_to_exact_size(egui::vec2(20.0, 20.0))); }
                                else { let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover()); ui.painter().circle_filled(r.center(), 4.0, MUTED); }
                                ui.label(RichText::new(format!("{name} ({count})")).strong());
                            });
                            row.col(|_| {});
                            row.col(|ui| { ui.label(format!("{cpu:.1}%")); });
                            row.col(|ui| { ui.label(format!("{:.1} MB", *memory as f64 / 1048576.0)); });
                            for _ in 0..2 { row.col(|_| {}); }
                        }
                        ProcessRow::Process(p) => {
                        let p = *p;
                        row.col(|ui| {
                            ui.allocate_exact_size(egui::vec2(14.0, 1.0), egui::Sense::hover());
                            if let Some(icon) = &p.icon { ui.add(egui::Image::new(icon).fit_to_exact_size(egui::vec2(20.0, 20.0))); }
                            else { let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover()); ui.painter().circle_filled(r.center(), 4.0, MUTED); }
                            ui.label(&p.name).on_hover_text(p.exe.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Executable path unavailable".into()));
                        });
                        row.col(|ui| { ui.label(p.pid.to_string()); });
                        row.col(|ui| { ui.label(format!("{:.1}%", p.cpu)); });
                        row.col(|ui| { ui.label(format!("{:.1} MB", p.memory as f64 / 1048576.0)); });
                        row.col(|ui| { ui.label(&p.status); });
                        row.col(|ui| { ui.horizontal(|ui| {
                            let dir = p.exe.as_ref().and_then(|p| p.parent());
                            if folder_button(ui, dir.is_some()).clicked() {
                                self.tx.send(Command::Open(dir.unwrap().to_owned())).ok();
                            }
                            if kill_button(ui, p.pid > 4 && p.pid != std::process::id() && p.start != 0).clicked() { self.confirmation = Some(p.clone()); }
                        }); });
                        }
                        }
                    });
                });
        });
        if let Some(p) = self.confirmation.clone() {
            egui::Modal::new(egui::Id::new("end_process")).show(ctx, |ui| {
                ui.heading("End process?");
                ui.label(format!("{} (PID {})", p.name, p.pid));
                ui.label("This forcibly stops the process. Unsaved work may be lost.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.confirmation = None;
                    }
                    if ui.button(RichText::new("End process").color(RED)).clicked() {
                        self.tx.send(Command::Kill(p)).ok();
                        self.confirmation = None;
                    }
                });
            });
        }
    }
}
fn sorted_process_rows<'a>(
    rows: &[&'a Process],
    sort: usize,
    descending: bool,
    expanded: &std::collections::HashSet<u32>,
    query: &str,
) -> Vec<ProcessRow<'a>> {
    sorted_process_rows_for_platform(rows, sort, descending, expanded, std::env::consts::OS, query)
}

fn sorted_process_rows_for_platform<'a>(
    rows: &[&'a Process],
    sort: usize,
    descending: bool,
    expanded: &std::collections::HashSet<u32>,
    platform: &str,
    query: &str,
) -> Vec<ProcessRow<'a>> {
    let by_id: std::collections::HashMap<_, _> = rows.iter().map(|p| (p.pid, *p)).collect();
    let explorer_ids: std::collections::HashSet<u32> = rows.iter()
        .filter(|p| p.name.eq_ignore_ascii_case("explorer.exe"))
        .map(|p| p.pid).collect();
    let system_names = ["system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "services.exe", "lsass.exe", "winlogon.exe", "svchost.exe"];
    let system_ids: std::collections::HashSet<u32> = rows.iter()
        .filter(|p| p.pid <= 4 || system_names.iter().any(|name| p.name.eq_ignore_ascii_case(name)))
        .map(|p| p.pid).collect();
    let mut explorer_members = explorer_ids.clone();
    let mut system_members = system_ids.clone();
    for p in rows {
        let mut parent = p.parent;
        let mut seen = std::collections::HashSet::new();
        while let Some(pid) = parent {
            if !seen.insert(pid) { break; }
            if explorer_ids.contains(&pid) { explorer_members.insert(p.pid); break; }
            parent = by_id.get(&pid).and_then(|parent| parent.parent);
        }
        let mut parent = p.parent;
        seen.clear();
        while let Some(pid) = parent {
            if !seen.insert(pid) { break; }
            if system_ids.contains(&pid) { system_members.insert(p.pid); break; }
            parent = by_id.get(&pid).and_then(|parent| parent.parent);
        }
    }
    if platform != "windows" {
        explorer_members = rows.iter().filter(|p| p.same_user == Some(true)).map(|p| p.pid).collect();
        system_members = rows.iter().filter(|p| p.same_user == Some(false) || p.pid <= 1).map(|p| p.pid).collect();
    }
    let other_members: std::collections::HashSet<u32> = rows.iter()
        .filter(|p| !explorer_members.contains(&p.pid) && !system_members.contains(&p.pid))
        .map(|p| p.pid).collect();
    system_members.retain(|pid| !explorer_members.contains(pid));

    let mut app_groups: std::collections::BTreeMap<(usize, String), Vec<&Process>> = std::collections::BTreeMap::new();
    let section_members = [&explorer_members, &system_members, &other_members];
    for p in rows {
        let text = format!("{} {} {}", p.name, p.pid, p.exe.as_ref().map(|path| path.display().to_string()).unwrap_or_default()).to_lowercase();
        if !query.split_whitespace().all(|word| text.contains(word)) {
            continue;
        }
        let section = section_members.iter().position(|members| members.contains(&p.pid)).unwrap_or(2);
        app_groups.entry((section, p.name.to_lowercase())).or_default().push(*p);
    }
    let mut sections: [Vec<ProcessGroup<'a>>; 3] = std::array::from_fn(|_| Vec::new());
    for mut instances in app_groups.into_values() {
        // Section membership must not change when the sort column changes.
        instances.sort_by_key(|p| p.pid);
        let section = instances.iter().find_map(|p| section_members.iter().position(|members| members.contains(&p.pid))).unwrap_or(2);
        let first = instances[0];
        let row = if instances.len() > 1 {
            ProcessRow::AppGroup {
                name: first.name.clone(),
                pid: first.pid,
                count: instances.len(),
                icon: instances.iter().find_map(|p| p.icon.clone()),
                cpu: instances.iter().map(|p| p.cpu).sum(),
                memory: instances.iter().map(|p| p.memory).sum(),
            }
        } else {
            ProcessRow::Process(first)
        };
        instances.sort_by(|a, b| compare_process_rows(&ProcessRow::Process(a), &ProcessRow::Process(b), sort, descending));
        sections[section].push((row, instances));
    }
    let mut ordered = Vec::new();
    let titles = [if platform == "windows" { "Explorer" } else { "User processes" }, "System", "Other"];
    for (title, mut groups) in titles.into_iter().zip(sections) {
        groups.sort_by(|a, b| compare_process_rows(&a.0, &b.0, sort, descending));
        ordered.push(ProcessRow::Section(title));
        for (row, instances) in groups {
            let show_children = matches!(&row, ProcessRow::AppGroup { pid, .. } if expanded.contains(pid));
            ordered.push(row);
            if show_children {
                ordered.extend(instances.into_iter().map(ProcessRow::Process));
            }
        }
    }
    ordered
}

fn compare_process_rows(a: &ProcessRow<'_>, b: &ProcessRow<'_>, sort: usize, descending: bool) -> std::cmp::Ordering {
    fn values<'a>(row: &'a ProcessRow<'_>) -> (&'a str, u32, f32, u64, &'a str) {
        match row {
            ProcessRow::AppGroup { name, pid, cpu, memory, .. } => (name, *pid, *cpu, *memory, ""),
            ProcessRow::Process(p) => (&p.name, p.pid, p.cpu, p.memory, &p.status),
            ProcessRow::Section(name) => (name, 0, 0.0, 0, ""),
        }
    }
    let a = values(a);
    let b = values(b);
    let order = match sort {
        1 => a.1.cmp(&b.1),
        2 => a.2.total_cmp(&b.2),
        3 => a.3.cmp(&b.3),
        4 => a.4.to_lowercase().cmp(&b.4.to_lowercase()),
        _ => a.0.to_lowercase().cmp(&b.0.to_lowercase()),
    }.then(a.1.cmp(&b.1));
    if descending { order.reverse() } else { order }
}

fn folder_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new("").min_size(egui::vec2(30.0, 30.0)),
        )
        .on_hover_text("Open executable folder");
    let r = response.rect.shrink(8.0);
    let color = if enabled { CYAN } else { MUTED };
    let p = ui.painter();
    p.rect_filled(
        egui::Rect::from_min_size(
            r.left_top() + egui::vec2(0.0, 1.0),
            egui::vec2(r.width() * 0.48, 3.0),
        ),
        1.0,
        color,
    );
    p.rect_stroke(
        egui::Rect::from_min_max(r.left_top() + egui::vec2(0.0, 3.0), r.right_bottom()),
        1.0,
        egui::Stroke::new(1.2_f32, color),
        egui::StrokeKind::Inside,
    );
    response
}
fn kill_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new("").min_size(egui::vec2(30.0, 30.0)),
        )
        .on_hover_text("End process");
    let r = response.rect.shrink(11.0);
    let p = ui.painter();
    p.line_segment(
        [r.left_top(), r.right_bottom()],
        egui::Stroke::new(1.5_f32, RED),
    );
    p.line_segment(
        [r.right_top(), r.left_bottom()],
        egui::Stroke::new(1.5_f32, RED),
    );
    response
}
fn kill(p: &Process) -> Result<String, String> {
    if p.pid <= 4 || p.pid == std::process::id() {
        return Err("This process is protected by VijiDeck.".into());
    }
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(p.pid)]), true);
    let current = system
        .process(Pid::from_u32(p.pid))
        .ok_or("The process has already exited.")?;
    if p.start == 0 || current.start_time() != p.start {
        return Err("Process identity changed. Refresh and try again.".into());
    }
    if crate::ports::force_kill(current, p.start)? {
        Ok(format!("Kill request sent to {} (PID {}).", p.name, p.pid))
    } else {
        Err("The OS rejected the request. Administrator permissions may be required.".into())
    }
}
fn open_folder(path: &std::path::Path) -> Result<String, String> {
    if !path.is_dir() {
        return Err("The executable folder no longer exists or is inaccessible.".into());
    }
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(path)
        .spawn()
        .map_err(|e| format!("Could not open folder: {e}"))?;
    Ok(format!("Opened {}", path.display()))
}

#[cfg(not(windows))]
fn executable_icon(_: &std::path::Path) -> Option<egui::ColorImage> {
    None
}

#[cfg(windows)]
fn executable_icon(path: &std::path::Path) -> Option<egui::ColorImage> {
    use std::{
        mem::{size_of, zeroed},
        os::windows::ffi::OsStrExt,
        ptr::null_mut,
    };
    use windows_sys::Win32::{
        Graphics::Gdi::*,
        UI::{Shell::*, WindowsAndMessaging::*},
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: All pointers refer to initialized buffers; each acquired native resource
    // is released before returning, and pixels are copied while the bitmap is live.
    unsafe {
        let mut info: SHFILEINFOW = zeroed();
        if SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        ) == 0
            || info.hIcon.is_null()
        {
            return None;
        }
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            DestroyIcon(info.hIcon);
            return None;
        }
        let mut bmi: BITMAPINFO = zeroed();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = 32;
        bmi.bmiHeader.biHeight = -32;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(dc);
            DestroyIcon(info.hIcon);
            return None;
        }
        let old = SelectObject(dc, bitmap);
        // Render on the opaque row background so legacy mask-only icons also display correctly.
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), 32 * 32 * 4);
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[18, 13, 8, 255]);
        }
        let drawn = DrawIconEx(dc, 0, 0, info.hIcon, 32, 32, 0, null_mut(), DI_NORMAL);
        GdiFlush();
        let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), 32 * 32 * 4);
        let mut rgba = Vec::with_capacity(pixels.len());
        for pixel in pixels.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        DestroyIcon(info.hIcon);
        (drawn != 0).then(|| egui::ColorImage::from_rgba_unmultiplied([32, 32], &rgba))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn process(pid: u32, name: &str, cpu: f32, memory: u64) -> Process {
        Process {
            pid, parent: None, same_user: None, start: 1, name: name.into(), exe: None,
            cpu, memory, status: "Running".into(), icon: None,
        }
    }

    fn visible_pids(rows: &[ProcessRow<'_>]) -> Vec<u32> {
        rows.iter().filter_map(|row| match row {
            ProcessRow::AppGroup { pid, .. } => Some(*pid),
            ProcessRow::Process(p) => Some(p.pid),
            ProcessRow::Section(_) => None,
        }).collect()
    }

    #[test]
    fn clicking_headers_changes_sort_and_toggles_direction() {
        let ctx = egui::Context::default();
        let (tx, _commands) = mpsc::channel();
        let (_events, rx) = mpsc::channel();
        let mut view = Processes {
            tx, rx, rows: vec![process(10, "Beta", 10.0, 100)],
            query: String::new(), sort: 0, descending: false,
            auto: false, busy: false, last: Instant::now(), notice: None,
            confirmation: None, expanded: HashSet::new(),
        };
        let frame = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            events,
            ..Default::default()
        };
        for (column, name) in ["Name", "PID", "CPU", "Memory", "Status"].into_iter().enumerate() {
            let output = ctx.run(frame(vec![]), |ctx| view.show(ctx));
            let pos = output.shapes.iter().find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && (text.galley.text() == name
                        || text.galley.text().starts_with(&format!("{name} ▲"))
                        || text.galley.text().starts_with(&format!("{name} ▼")))
                {
                    return Some(text.pos + text.galley.size() * 0.5);
                }
                None
            }).expect("column header");
            for click in 0..2 {
                for pressed in [true, false] {
                    let _ = ctx.run(frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos, button: egui::PointerButton::Primary, pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]), |ctx| view.show(ctx));
                }
                assert_eq!(view.sort, column);
                let initial_descending = column != 1 && column != 4;
                assert_eq!(view.descending, initial_descending ^ (click == 1));
            }
        }
    }

    #[test]
    fn headers_sort_groups_by_displayed_totals_in_both_directions() {
        // The group total exceeds the singleton, but each member is smaller.
        let processes = [
            process(20, "Alpha", 6.0, 60),
            process(30, "alpha", 6.0, 60),
            process(10, "Beta", 10.0, 100),
        ];
        let refs: Vec<_> = processes.iter().collect();
        for (sort, ascending) in [(0, vec![20, 10]), (1, vec![10, 20]), (2, vec![10, 20]), (3, vec![10, 20])] {
            let rows = sorted_process_rows(&refs, sort, false, &HashSet::new(), "");
            assert_eq!(visible_pids(&rows), ascending, "column {sort} ascending");
            let rows = sorted_process_rows(&refs, sort, true, &HashSet::new(), "");
            assert_eq!(visible_pids(&rows), ascending.into_iter().rev().collect::<Vec<_>>(), "column {sort} descending");
        }
    }

    #[test]
    fn worker_reads_current_process_ownership() {
        let view = Processes::new(egui::Context::default());
        view.tx.send(Command::Scan).unwrap();
        let Event::Rows(rows) = view.rx.recv_timeout(Duration::from_secs(30)).unwrap() else {
            panic!("Expected a process inventory");
        };
        let current = rows.iter().find(|p| p.pid == std::process::id()).expect("current process");
        assert_eq!(current.same_user, Some(true));
    }

    #[test]
    fn windows_search_keeps_ancestry_even_when_explorer_is_filtered_out() {
        let mut app = process(20, "Editor", 1.0, 20);
        app.parent = Some(100);
        let explorer = process(100, "explorer.exe", 0.0, 10);
        let rows = sorted_process_rows_for_platform(&[&app, &explorer], 0, false, &HashSet::new(), "windows", "editor");
        assert_eq!(visible_pids(&rows), vec![20]);
        assert!(matches!(rows[0], ProcessRow::Section("Explorer")));
        assert!(matches!(rows[1], ProcessRow::Process(p) if p.pid == 20));
    }

    #[test]
    fn unix_groups_user_processes_without_explorer_and_separates_service_instances() {
        let mut processes = [
            process(20, "python", 1.0, 20),
            process(30, "python", 2.0, 30),
            process(40, "unknown", 0.0, 10),
        ];
        processes[0].same_user = Some(true);
        processes[1].same_user = Some(false);
        let refs: Vec<_> = processes.iter().collect();
        for platform in ["linux", "macos"] {
            for sort in 0..5 {
                for descending in [false, true] {
                    let rows = sorted_process_rows_for_platform(&refs, sort, descending, &HashSet::new(), platform, "");
                    let mut section = "";
                    let mut found = 0;
                    for row in rows {
                        match row {
                            ProcessRow::Section(title) => {
                                assert_ne!(title, "Explorer");
                                section = title;
                            }
                            ProcessRow::Process(p) => {
                                assert_eq!(section, match p.pid {
                                    20 => "User processes", 30 => "System", _ => "Other",
                                });
                                found += 1;
                            }
                            ProcessRow::AppGroup { .. } => panic!("Different owners must not share an app group"),
                        }
                    }
                    assert_eq!(found, 3);
                }
            }
        }
    }

    #[test]
    fn status_header_sorts_by_status_text() {
        let mut processes = [process(20, "Alpha", 0.0, 0), process(10, "Beta", 0.0, 0)];
        processes[0].status = "Sleeping".into();
        let refs: Vec<_> = processes.iter().collect();
        assert_eq!(visible_pids(&sorted_process_rows(&refs, 4, false, &HashSet::new(), "")), vec![10, 20]);
        assert_eq!(visible_pids(&sorted_process_rows(&refs, 4, true, &HashSet::new(), "")), vec![20, 10]);
    }

    #[test]
    fn expanded_children_stay_with_group_and_follow_sort() {
        let processes = [
            process(20, "Alpha", 4.0, 40),
            process(30, "Alpha", 8.0, 80),
            process(10, "Beta", 10.0, 100),
        ];
        let refs: Vec<_> = processes.iter().collect();
        let expanded = HashSet::from([20]);
        for sort in [2, 3] {
            assert_eq!(visible_pids(&sorted_process_rows(&refs, sort, true, &expanded, "")), vec![20, 30, 20, 10]);
            assert_eq!(visible_pids(&sorted_process_rows(&refs, sort, false, &expanded, "")), vec![10, 20, 20, 30]);
        }
    }

    #[test]
    fn sorting_keeps_section_membership_and_expansion_identity_stable() {
        let mut processes = [
            process(20, "Alpha", 1.0, 20),
            process(30, "Alpha", 9.0, 90),
            process(100, "explorer.exe", 0.0, 10),
        ];
        processes[0].parent = Some(100);
        processes[1].parent = Some(100);
        let refs: Vec<_> = processes.iter().collect();
        for sort in 0..4 {
            for descending in [false, true] {
                let rows = sorted_process_rows_for_platform(&refs, sort, descending, &HashSet::from([20]), "windows", "");
                let mut section = "";
                for row in rows {
                    match row {
                        ProcessRow::Section(title) => section = title,
                        ProcessRow::AppGroup { pid, .. } => {
                            assert_eq!(pid, 20);
                            assert_eq!(section, "Explorer");
                        }
                        ProcessRow::Process(_) => assert_eq!(section, "Explorer"),
                    }
                }
            }
        }
    }
}
