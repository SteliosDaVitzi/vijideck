use crate::ui::*;
use eframe::egui::{self, Color32, RichText, Stroke};
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use sysinfo::{Components, Disks, Networks, System};

#[derive(Default)]
struct Sample {
    cpu: f32,
    gpu: Option<f32>,
    threads: Vec<f32>,
    processor: String,
    cores: Option<usize>,
    frequency: u64,
    memory: u64,
    total_memory: u64,
    swap: u64,
    total_swap: u64,
    disks: Vec<Volume>,
    networks: Vec<(String, String, f64, f64)>,
    temperatures: Vec<(String, f32)>,
    uptime: u64,
}
struct Volume {
    name: String,
    mount: String,
    kind: String,
    filesystem: String,
    total: u64,
    free: u64,
}
struct Hardware {
    sections: Vec<(String, String)>,
    gpu_card: Option<(String, String)>,
    note: Option<String>,
}

impl Hardware {
    fn graphics_details(&self) -> Option<&str> {
        self.sections.iter().find_map(|(title, value)| {
            let value = value.trim();
            (title == "Graphics"
                && !value.is_empty()
                && !value.eq_ignore_ascii_case("Unavailable")
                && !value.eq_ignore_ascii_case("Not reported"))
            .then_some(value)
        })
    }
}

fn show_gpu(usage: Option<f32>, hardware: Option<&Hardware>) -> bool {
    // Wait for inventory before deciding there is no GPU information. An idle
    // GPU (0%) or known adapter without counters should still have a GPU gauge.
    hardware.is_none()
        || usage.is_some()
        || hardware.is_some_and(|h| h.gpu_card.is_some() || h.graphics_details().is_some())
}

pub struct Overview {
    rx: Receiver<Sample>,
    hardware_rx: Receiver<Hardware>,
    sample: Option<Sample>,
    hardware: Option<Hardware>,
    history: VecDeque<f32>,
    updated: Instant,
}

impl Overview {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        let (hardware_tx, hardware_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hardware-inventory".into())
            .spawn(move || {
                hardware_tx.send(hardware_inventory()).ok();
            })
            .expect("start hardware inventory");
        std::thread::Builder::new()
            .name("system-monitor".into())
            .spawn(move || {
                let mut system = System::new();
                let mut gpu = crate::gpu::Monitor::new();
                let mut disks = Disks::new_with_refreshed_list();
                let mut networks = Networks::new_with_refreshed_list();
                let mut components = Components::new_with_refreshed_list();
                system.refresh_cpu_all();
                let cores = System::physical_core_count();
                let mut last = Instant::now();
                std::thread::sleep(Duration::from_millis(300));
                loop {
                    system.refresh_cpu_all();
                    system.refresh_memory();
                    disks.refresh(true);
                    networks.refresh(true);
                    components.refresh(true);
                    let elapsed = last.elapsed().as_secs_f64().max(0.001);
                    last = Instant::now();
                    let mut network_rows: Vec<_> = networks
                        .iter()
                        .map(|(name, data)| {
                            (
                                name.clone(),
                                format!(
                                    "{}  {}",
                                    data.mac_address(),
                                    data.ip_networks()
                                        .iter()
                                        .map(|ip| ip.to_string())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                                data.received() as f64 / elapsed,
                                data.transmitted() as f64 / elapsed,
                            )
                        })
                        .collect();
                    network_rows.sort_by(|a, b| a.0.cmp(&b.0));
                    let sample = Sample {
                        cpu: system.global_cpu_usage(),
                        gpu: gpu.sample(),
                        threads: system.cpus().iter().map(|cpu| cpu.cpu_usage()).collect(),
                        processor: system
                            .cpus()
                            .first()
                            .map(|cpu| cpu.brand().trim().to_owned())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "Processor name unavailable".into()),
                        cores,
                        frequency: system.cpus().first().map_or(0, |cpu| cpu.frequency()),
                        memory: system.used_memory(),
                        total_memory: system.total_memory(),
                        swap: system.used_swap(),
                        total_swap: system.total_swap(),
                        disks: disks
                            .iter()
                            .map(|disk| Volume {
                                name: disk.name().to_string_lossy().into_owned(),
                                mount: disk.mount_point().display().to_string(),
                                kind: format!("{:?}", disk.kind()),
                                filesystem: disk.file_system().to_string_lossy().into_owned(),
                                total: disk.total_space(),
                                free: disk.available_space(),
                            })
                            .collect(),
                        networks: network_rows,
                        temperatures: components
                            .iter()
                            .filter_map(|c| c.temperature().map(|t| (c.label().to_owned(), t)))
                            .collect(),
                        uptime: System::uptime(),
                    };
                    match tx.try_send(sample) {
                        Err(mpsc::TrySendError::Disconnected(_)) => break,
                        Err(mpsc::TrySendError::Full(_)) | Ok(()) => {}
                    }
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_secs(1));
                }
            })
            .expect("start system monitor");
        Self {
            rx,
            hardware_rx,
            sample: None,
            hardware: None,
            history: VecDeque::new(),
            updated: Instant::now(),
        }
    }

    pub fn poll(&mut self) {
        while let Ok(sample) = self.rx.try_recv() {
            self.history.push_back(sample.cpu);
            if self.history.len() > 60 {
                self.history.pop_front();
            }
            self.sample = Some(sample);
            self.updated = Instant::now();
        }
        if let Ok(hardware) = self.hardware_rx.try_recv() {
            self.hardware = Some(hardware);
        }
    }

    pub fn status(&self) -> &str {
        if self.sample.is_none() {
            "Reading system hardware"
        } else if self.updated.elapsed() > Duration::from_secs(5) {
            "Telemetry delayed"
        } else {
            "Live monitoring / 1 second samples"
        }
    }

    pub fn widget(&self, ctx: &egui::Context, can_hide: bool) -> Option<crate::background::Action> {
        use crate::background::Action;
        let mut action = None;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(11, 18, 24, 225))
                    .stroke(Stroke::new(1.0_f32, LINE))
                    .corner_radius(16)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                let drag_area = ui.interact(
                    ui.max_rect(),
                    ui.id().with("widget_drag_area"),
                    egui::Sense::click_and_drag(),
                );
                if drag_area.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("VIJIDECK").size(10.0).strong().color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if can_hide && Self::minimize_button(ui).clicked() {
                            action = Some(Action::Hide);
                        }
                    });
                });
                ui.add_space(5.0);
                let values = self
                    .sample
                    .as_ref()
                    .map(|sample| {
                        [
                            Some(sample.cpu),
                            (sample.total_memory > 0)
                                .then(|| sample.memory as f32 / sample.total_memory as f32 * 100.0),
                            sample.gpu,
                        ]
                    })
                    .unwrap_or([None; 3]);
                ui.columns(3, |columns| {
                    for ((ui, label), value) in
                        columns.iter_mut().zip(["CPU", "RAM", "GPU"]).zip(values)
                    {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(label).size(9.0).color(MUTED));
                            Self::mini_gauge(ui, value);
                        });
                    }
                });
                ui.label(
                    RichText::new(if self.sample.is_none() {
                        "Waiting for telemetry"
                    } else if self.updated.elapsed() > Duration::from_secs(5) {
                        "Telemetry delayed"
                    } else {
                        "Live · 1s"
                    })
                    .size(9.0)
                    .color(MUTED),
                );
            });
        action
    }

    fn mini_gauge(ui: &mut egui::Ui, value: Option<f32>) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(62.0, 62.0), egui::Sense::hover());
        let center = rect.center();
        let radius = 24.0;
        ui.painter()
            .circle_stroke(center, radius, Stroke::new(4.0_f32, LINE));
        if let Some(value) = value {
            let value = value.clamp(0.0, 100.0);
            let points: Vec<_> = (0..=48)
                .map(|i| {
                    center
                        + egui::Vec2::angled(
                            -std::f32::consts::FRAC_PI_2
                                + i as f32 / 48.0 * std::f32::consts::TAU * value / 100.0,
                        ) * radius
                })
                .collect();
            ui.painter()
                .add(egui::Shape::line(points, Stroke::new(4.0_f32, CYAN)));
        }
        ui.painter().text(
            center,
            egui::Align2::CENTER_CENTER,
            value.map_or_else(|| "—".into(), |v| format!("{v:.0}%")),
            egui::FontId::proportional(14.0),
            TEXT,
        );
    }

    fn minimize_button(ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 24.0), egui::Sense::click());
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            7,
            visuals.bg_fill,
            Stroke::new(
                1.0_f32,
                if response.hovered() {
                    CYAN.gamma_multiply(0.65)
                } else {
                    LINE
                },
            ),
            egui::StrokeKind::Inside,
        );
        let center = rect.center();
        ui.painter().line_segment(
            [
                center + egui::vec2(-4.0, 2.0),
                center + egui::vec2(4.0, 2.0),
            ],
            Stroke::new(1.6_f32, if response.hovered() { CYAN } else { TEXT }),
        );
        response.on_hover_text("Minimize to tray")
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.sample.is_some() && self.hardware.is_some() && self.history.len() >= 2
    }

    pub fn show(&self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(panel(BG, 28)).show(ctx, |ui| {
            let scroll = egui::ScrollArea::vertical().id_salt("overview_scroll").auto_shrink([false, false]);
            #[cfg(feature = "ui-preview")]
            let scroll = if std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref() == Ok("overview-hardware") {
                scroll.vertical_scroll_offset(660.0)
            } else { scroll };
            scroll.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading("System overview");
                        caption(ui, &format!("{}  /  {}  /  {}", System::host_name().unwrap_or_else(|| "This computer".into()), System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into()), System::cpu_arch()));
                    });
                });
                ui.add_space(22.0);
                let Some(s) = &self.sample else {
                    ui.spinner();
                    ui.label("Reading processor, memory, storage, and network activity…");
                    return;
                };
                ui.columns(3, |cols| {
                    gauge(&mut cols[0], "Processor", Some(s.cpu), CYAN, "Total CPU utilization", &format!("{} logical processors", s.threads.len()));
                    gauge(&mut cols[1], "Memory", percent(s.memory, s.total_memory), CYAN, &format!("{} in use", bytes(s.memory)), &format!("{} usable memory", bytes(s.total_memory)));
                    if show_gpu(s.gpu, self.hardware.as_ref()) {
                        let card = self.hardware.as_ref().and_then(|h| h.gpu_card.as_ref());
                        let model = card.map(|(model, _)| model.as_str()).unwrap_or(if self.hardware.is_none() { "Reading GPU model…" } else { "GPU model unavailable" });
                        let capacity = card.map(|(_, capacity)| capacity.as_str()).unwrap_or("Capacity unavailable");
                        gauge(&mut cols[2], "GPU", s.gpu, CYAN, model, capacity);
                    } else {
                        let total: u64 = s.disks.iter().map(|d| d.total).sum();
                        let free: u64 = s.disks.iter().map(|d| d.free).sum();
                        gauge(&mut cols[2], "Storage", percent(total.saturating_sub(free), total), AMBER, &format!("{} free", bytes(free)), &format!("{} across {} volumes", bytes(total), s.disks.len()));
                    }
                });
                ui.add_space(18.0);
                ui.horizontal_wrapped(|ui| {
                    badge(ui, &format!("Uptime {}d {}h {}m", s.uptime / 86400, s.uptime / 3600 % 24, s.uptime / 60 % 60), MUTED);
                    badge(ui, &format!("Swap {} / {}", bytes(s.swap), bytes(s.total_swap)), MUTED);
                    for (label, temperature) in &s.temperatures {
                        badge(ui, &format!("{label}  {temperature:.0} °C"), AMBER);
                    }
                });
                ui.add_space(24.0);
                if ui.available_width() >= 1000.0 {
                    ui.columns(2, |cols| {
                        self.processor(&mut cols[0], s);
                        self.storage(&mut cols[1], s);
                    });
                } else {
                    self.processor(ui, s);
                    ui.add_space(18.0);
                    self.storage(ui, s);
                }
                ui.add_space(24.0);
                section(ui, "Hardware inventory");
                caption(ui, "Device details reported by this computer");
                ui.add_space(12.0);
                if let Some(hardware) = &self.hardware {
                    if hardware.sections.is_empty() {
                        caption(ui, "Additional device details unavailable.");
                    }
                    for pair in hardware.sections.chunks(2) {
                        ui.columns(2, |cols| {
                            for (col, (title, value)) in cols.iter_mut().zip(pair) {
                                field(col, title, value);
                            }
                        });
                        ui.add_space(8.0);
                    }
                    if let Some(note) = &hardware.note { caption(ui, note); }
                } else {
                    ui.horizontal(|ui| { ui.spinner(); caption(ui, "Reading graphics, motherboard, memory modules, and connected devices…"); });
                }
                ui.add_space(24.0);
                section(ui, "Network interfaces");
                caption(ui, "Receive / send rates include virtual and loopback interfaces; they are not internet speed tests.");
                ui.add_space(10.0);
                if s.networks.is_empty() { caption(ui, "No network interfaces reported."); }
                for (name, address, received, sent) in &s.networks {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(format!("Receive {}/s    Send {}/s", bytes(*received as u64), bytes(*sent as u64))).color(CYAN));
                    });
                    ui.add(egui::Label::new(RichText::new(address).size(11.0).color(MUTED)).wrap().selectable(true));
                    ui.add_space(8.0);
                }
                ui.add_space(10.0);
                caption(ui, "Unavailable sensors and device details depend on operating system and driver support.");
            });
        });
    }

    fn processor(&self, ui: &mut egui::Ui, s: &Sample) {
        section(ui, "Processor activity");
        ui.add(
            egui::Label::new(RichText::new(&s.processor).size(16.0))
                .wrap()
                .selectable(true),
        );
        caption(
            ui,
            &format!(
                "{} physical cores / {} threads / {}",
                s.cores.map_or_else(|| "Unknown".into(), |n| n.to_string()),
                s.threads.len(),
                if s.frequency == 0 {
                    "Frequency unavailable".into()
                } else {
                    format!("{:.2} GHz reported", s.frequency as f64 / 1000.0)
                }
            ),
        );
        ui.add_space(8.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 65.0), egui::Sense::hover());
        let p = ui.painter();
        for fraction in [0.0, 0.5, 1.0] {
            let y = rect.bottom() - rect.height() * fraction;
            p.hline(rect.x_range(), y, Stroke::new(1.0_f32, LINE));
        }
        let points: Vec<_> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, cpu)| {
                egui::pos2(
                    rect.right() - (self.history.len() - 1 - i) as f32 / 59.0 * rect.width(),
                    rect.bottom() - cpu.clamp(0.0, 100.0) / 100.0 * rect.height(),
                )
            })
            .collect();
        if points.len() > 1 {
            p.add(egui::Shape::line(points, Stroke::new(1.5_f32, CYAN)));
        }
        caption(ui, "CPU history / up to 60 samples / 0–100%");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (i, usage) in s.threads.iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(14.0, 26.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, SURFACE);
                let fill = egui::Rect::from_min_max(
                    egui::pos2(
                        rect.left(),
                        rect.bottom() - rect.height() * usage.clamp(0.0, 100.0) / 100.0,
                    ),
                    rect.right_bottom(),
                );
                ui.painter().rect_filled(fill, 2.0, CYAN);
                response.on_hover_text(format!("Logical processor {}: {:.1}%", i + 1, usage));
            }
        });
    }

    fn storage(&self, ui: &mut egui::Ui, s: &Sample) {
        section(ui, "Mounted storage");
        if s.disks.is_empty() {
            caption(ui, "No mounted volumes reported.");
        }
        for disk in &s.disks {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&disk.mount).strong());
                caption(
                    ui,
                    &format!("{} / {} / {}", disk.name, disk.kind, disk.filesystem),
                );
            });
            ui.add(
                egui::ProgressBar::new(
                    percent(disk.total.saturating_sub(disk.free), disk.total).unwrap_or(0.0)
                        / 100.0,
                )
                .fill(AMBER)
                .desired_height(5.0),
            );
            caption(
                ui,
                &format!("{} free of {}", bytes(disk.free), bytes(disk.total)),
            );
            ui.add_space(8.0);
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    rule(ui);
    ui.add_space(10.0);
    ui.label(RichText::new(title).size(18.0));
    ui.add_space(6.0);
}
fn percent(used: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32)
}
fn bytes(value: u64) -> String {
    if value >= 1 << 40 {
        format!("{:.1} TiB", value as f64 / (1u64 << 40) as f64)
    } else if value >= 1 << 30 {
        format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64)
    } else if value >= 1 << 20 {
        format!("{:.1} MiB", value as f64 / (1u64 << 20) as f64)
    } else if value >= 1 << 10 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}
fn gauge(
    ui: &mut egui::Ui,
    title: &str,
    usage: Option<f32>,
    color: Color32,
    detail: &str,
    subtitle: &str,
) {
    panel(PANEL, 16)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(5)
        .show(ui, |ui| {
            ui.set_min_width((ui.available_width()).max(0.0));
            ui.label(RichText::new(title).size(16.0));
            let size = ui.available_width().clamp(100.0, 165.0);
            ui.vertical_centered(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                let center = rect.center();
                let radius = size * 0.41;
                ui.painter()
                    .circle_stroke(center, radius, Stroke::new(7.0_f32, LINE));
                if let Some(value) = usage {
                    let points: Vec<_> = (0..=100)
                        .map(|i| {
                            center
                                + egui::Vec2::angled(
                                    -std::f32::consts::FRAC_PI_2
                                        + i as f32 / 100.0 * std::f32::consts::TAU * value / 100.0,
                                ) * radius
                        })
                        .collect();
                    ui.painter()
                        .add(egui::Shape::line(points, Stroke::new(7.0_f32, color)));
                }
                ui.painter().text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    usage.map_or_else(|| "—".into(), |v| format!("{v:.0}%")),
                    egui::FontId::proportional(36.0),
                    TEXT,
                );
            });
            ui.label(RichText::new(detail).size(14.0));
            caption(ui, subtitle);
        });
}

pub fn rail_icon(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(44.0), egui::Sense::click());
    let active = selected || response.hovered() || response.has_focus();
    let p = ui.painter();
    p.rect_filled(rect, 5.0, if active { SURFACE } else { PANEL });
    if selected {
        p.line_segment(
            [
                rect.left_top() + egui::vec2(0.0, 9.0),
                rect.left_bottom() - egui::vec2(0.0, 9.0),
            ],
            Stroke::new(2.0_f32, CYAN),
        );
    }
    for (x, y) in [(-9.0, -9.0), (2.0, -9.0), (-9.0, 2.0), (2.0, 2.0)] {
        p.rect_stroke(
            egui::Rect::from_min_size(rect.center() + egui::vec2(x, y), egui::vec2(7.0, 7.0)),
            1.0,
            Stroke::new(1.3_f32, if active { CYAN } else { MUTED }),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text("System overview")
}

#[cfg(windows)]
fn hardware_inventory() -> Hardware {
    use std::{
        io::Read,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    // A fixed, read-only CIM query. No user content is interpolated into shell code.
    let script = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$ErrorActionPreference = 'Stop'
function Report($title, $class, $format) {
    try {
        $rows = @(Get-CimInstance -ClassName $class | ForEach-Object $format | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $value = if ($rows.Count) { $rows -join '; ' } else { 'Not reported' }
        $value = $value -replace '[\r\n\t]', ' '
        Write-Output ($title + "`t" + $value)
    } catch { Write-Output ($title + "`tUnavailable") }
}
Report 'Computer' 'Win32_ComputerSystem' { "$($_.Manufacturer) $($_.Model)" }
Report 'Motherboard' 'Win32_BaseBoard' { "$($_.Manufacturer) $($_.Product) / $($_.Version)" }
Report 'Graphics' 'Win32_VideoController' { "$($_.Name) / driver $($_.DriverVersion)" }
try {
    $drivers = @(Get-ChildItem -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}' -ErrorAction SilentlyContinue | ForEach-Object { Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue })
    $cards = @(Get-CimInstance -ClassName Win32_VideoController | Where-Object { $_.ConfigManagerErrorCode -eq 0 } | ForEach-Object {
        $name = $_.Name
        # AdapterRAM is only 32 bits; use the driver's 64-bit capacity instead.
        $memory = ($drivers | Where-Object { $_.DriverDesc -eq $name } | ForEach-Object { $_.'HardwareInformation.qwMemorySize' } | Measure-Object -Maximum).Maximum
        [PSCustomObject]@{ Name = $name; Memory = $memory }
    })
    $card = $cards | Sort-Object Memory -Descending | Select-Object -First 1
    if ($card) {
        $model = $card.Name -replace '[\r\n\t]', ' '
        $capacity = if ($card.Memory -gt 0) { "{0:0} GiB" -f ($card.Memory / 1GB) } else { 'Capacity unavailable' }
        Write-Output ("GPU card`t" + $model + "`t" + $capacity)
    }
} catch {}
Report 'Memory modules' 'Win32_PhysicalMemory' { "$($_.DeviceLocator): $([math]::Round($_.Capacity / 1GB, 1)) GiB / $($_.ConfiguredClockSpeed) MHz / $($_.Manufacturer) $($_.PartNumber.Trim())" }
Report 'Physical drives' 'Win32_DiskDrive' { "$($_.Model) / $([math]::Round($_.Size / 1GB, 1)) GiB / $($_.InterfaceType)" }
Report 'Firmware' 'Win32_BIOS' { "$($_.Manufacturer) / $($_.SMBIOSBIOSVersion)" }
Report 'Displays' 'Win32_DesktopMonitor' { "$($_.Name) / $($_.MonitorManufacturer)" }
Report 'Audio' 'Win32_SoundDevice' { $_.Name }
Report 'Network adapters' 'Win32_NetworkAdapter' { if ($_.PhysicalAdapter) { $_.Name } }
Report 'Battery' 'Win32_Battery' { "$($_.Name) / $($_.EstimatedChargeRemaining)% remaining" }
Report 'Keyboards' 'Win32_Keyboard' { $_.Name }
Report 'Pointing devices' 'Win32_PointingDevice' { $_.Name }
"#;
    let result = (|| -> Result<String, String> {
        // Drain the pipe concurrently so large inventories cannot block the provider.
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .creation_flags(0x08000000)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut stdout = child.stdout.take().ok_or("Hardware output unavailable")?;
        let reader = std::thread::spawn(move || {
            let mut output = Vec::new();
            stdout.read_to_end(&mut output).map(|_| output)
        });
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < Duration::from_secs(25) => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                _ => {
                    child.kill().ok();
                    child.wait().ok();
                    return Err("Hardware inventory timed out or could not be read.".into());
                }
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        let output = reader
            .join()
            .map_err(|_| "Hardware output reader failed")?
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Windows hardware inventory could not be read.".into());
        }
        Ok(String::from_utf8_lossy(&output).into_owned())
    })();
    match result {
        Ok(output) => Hardware {
            gpu_card: output.lines().find_map(|line| {
                let value = line.strip_prefix("GPU card\t")?;
                let (model, capacity) = value.split_once('\t')?;
                Some((model.to_owned(), capacity.to_owned()))
            }),
            sections: output
                .lines()
                .filter(|line| !line.starts_with("GPU card\t"))
                .filter_map(|line| {
                    line.split_once('\t')
                        .map(|(k, v)| (k.to_owned(), v.trim().to_owned()))
                })
                .collect(),
            note: Some(
                "GPU utilization uses supported Windows performance counters. VRAM and fan speeds are not collected."
                    .into(),
            ),
        },
        Err(note) => Hardware {
            gpu_card: None,
            sections: Vec::new(),
            note: Some(note),
        },
    }
}

#[cfg(not(windows))]
fn hardware_inventory() -> Hardware {
    Hardware { gpu_card: None, sections: vec![("Architecture".into(), System::cpu_arch()), ("Kernel".into(), System::kernel_version().unwrap_or_else(|| "Unavailable".into()))], note: Some("Extended graphics, motherboard, firmware, and memory module inventory is currently available on Windows. Processor, memory, mounted storage, network, and supported temperature sensors are shown above.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_percent_handles_missing_and_inconsistent_totals() {
        assert_eq!(percent(10, 0), None);
        assert_eq!(percent(25, 100), Some(25.0));
        assert_eq!(percent(110, 100), Some(100.0));
        assert_eq!(percent(u64::MAX, u64::MAX), Some(100.0));
    }

    #[test]
    fn gpu_gauge_falls_back_only_when_all_gpu_information_is_missing() {
        assert!(show_gpu(None, None), "inventory is still loading");
        for value in ["", "  ", "Unavailable", "Not reported"] {
            let hardware = Hardware {
                gpu_card: None,
                sections: vec![("Graphics".into(), value.into())],
                note: None,
            };
            assert!(!show_gpu(None, Some(&hardware)));
            assert!(
                show_gpu(Some(0.0), Some(&hardware)),
                "idle is a valid reading"
            );
        }
        let hardware = Hardware {
            gpu_card: None,
            sections: vec![("Graphics".into(), "Example GPU / driver 1.0".into())],
            note: None,
        };
        assert!(
            show_gpu(None, Some(&hardware)),
            "known GPU without utilization"
        );
        assert_eq!(
            hardware.graphics_details(),
            Some("Example GPU / driver 1.0")
        );
        let failed = Hardware {
            gpu_card: None,
            sections: Vec::new(),
            note: Some("Inventory failed".into()),
        };
        assert!(!show_gpu(None, Some(&failed)));
        assert!(
            show_gpu(Some(12.0), Some(&failed)),
            "counters work without inventory"
        );
    }
}
