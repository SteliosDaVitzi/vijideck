use crate::ui::*;
use eframe::egui::{self, RichText};
use std::{
    collections::VecDeque,
    io::Read,
    process::{Command, Stdio},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::Duration,
};

const MAX_LINES: usize = 2_000;
const MAX_LINE_BYTES: usize = 16_384;

#[derive(Default)]
struct Buffer {
    lines: VecDeque<(String, bool)>,
    finished: bool,
    error: Option<String>,
}
impl Buffer {
    fn push(&mut self, text: String, stderr: bool) {
        if self.lines.len() == MAX_LINES { self.lines.pop_front(); }
        self.lines.push_back((text, stderr));
    }
}

pub struct ContainerLogs {
    pub id: String,
    name: String,
    buffer: Arc<Mutex<Buffer>>,
    cancel: Arc<AtomicBool>,
    follow: bool,
}
impl Drop for ContainerLogs {
    fn drop(&mut self) { self.cancel.store(true, Ordering::Relaxed); }
}
impl ContainerLogs {
    pub fn new(id: String, name: String, ctx: egui::Context) -> Self {
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let logs = Self { id: id.clone(), name, buffer: buffer.clone(), cancel: cancel.clone(), follow: true };
        std::thread::spawn(move || {
            let result = stream(&id, &buffer, &cancel, &ctx);
            let mut data = buffer.lock().unwrap();
            data.finished = true;
            if let Err(error) = result { data.error = Some(error); }
            drop(data);
            ctx.request_repaint();
        });
        logs
    }
    /// Returns true when the user closes the drawer.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut close = false;
        egui::TopBottomPanel::bottom("docker_logs")
            .resizable(true).default_height(240.0).min_height(130.0)
            .max_height((ctx.available_rect().height() * 0.65).max(130.0))
            .frame(panel(PANEL, 16)).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    caption(ui, "CONTAINER LOGS");
                    ui.label(RichText::new(&self.name).strong().color(CYAN));
                    let data = self.buffer.lock().unwrap();
                    badge(ui, if data.finished { "ENDED" } else { "LIVE" }, if data.finished { MUTED } else { CYAN });
                    drop(data);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() { close = true; }
                        ui.checkbox(&mut self.follow, "Follow");
                        if ui.button("Clear").clicked() { self.buffer.lock().unwrap().lines.clear(); }
                    });
                });
                rule(ui);
                let data = self.buffer.lock().unwrap();
                if let Some(error) = &data.error { ui.colored_label(RED, error); }
                let lines: Vec<_> = data.lines.iter().cloned().collect();
                let finished = data.finished;
                drop(data);
                if lines.is_empty() { ui.label(RichText::new(if finished { "No log output." } else { "Waiting for log output…" }).color(MUTED)); }
                let height = ui.text_style_height(&egui::TextStyle::Monospace);
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::both().id_salt((&self.id, "output"))
                    .auto_shrink([false, false]).stick_to_bottom(self.follow)
                    .show_rows(ui, height, lines.len(), |ui, range| {
                        for (line, stderr) in &lines[range] {
                            ui.add(egui::Label::new(RichText::new(line).monospace().color(if *stderr { AMBER } else { TEXT }))
                                .selectable(true).wrap_mode(egui::TextWrapMode::Extend));
                        }
                    });
            });
        close
    }
}

fn stream(id: &str, buffer: &Arc<Mutex<Buffer>>, cancel: &Arc<AtomicBool>, ctx: &egui::Context) -> Result<(), String> {
    let mut command = Command::new("docker");
    command.args(["logs", "--follow", "--tail", "200", "--timestamps", id])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    stream_command(command, buffer, cancel, ctx)
}

fn stream_command(mut command: Command, buffer: &Arc<Mutex<Buffer>>, cancel: &Arc<AtomicBool>, ctx: &egui::Context) -> Result<(), String> {
    let mut child = command.spawn().map_err(|e| format!("Cannot open Docker logs: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out_buffer = buffer.clone();
    let out_ctx = ctx.clone();
    let out = std::thread::spawn(move || read_output(stdout, false, &out_buffer, &out_ctx));
    let err_buffer = buffer.clone();
    let err_ctx = ctx.clone();
    let err = std::thread::spawn(move || read_output(stderr, true, &err_buffer, &err_ctx));
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            child.kill().ok(); child.wait().ok(); break Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => break if status.success() { Ok(()) } else { Err("Docker log stream ended with an error. See output above; reopen Logs to retry.".into()) },
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => { child.kill().ok(); child.wait().ok(); break Err(format!("Docker log stream failed: {error}")); }
        }
    };
    let stdout_result = out.join().map_err(|_| "Log reader failed")?;
    let stderr_result = err.join().map_err(|_| "Log reader failed")?;
    stdout_result.and(stderr_result).and(result)
}

// Fixed-size reads bound memory even for a container emitting an unbroken line.
fn read_output(mut reader: impl Read, stderr: bool, buffer: &Arc<Mutex<Buffer>>, ctx: &egui::Context) -> Result<(), String> {
    let mut chunk = [0; 4096];
    let mut pending = Vec::new();
    loop {
        let count = reader.read(&mut chunk).map_err(|e| format!("Cannot read container logs: {e}"))?;
        if count == 0 { break; }
        let mut data = buffer.lock().unwrap();
        for &byte in &chunk[..count] {
            if byte == b'\n' || pending.len() >= MAX_LINE_BYTES {
                let text = String::from_utf8_lossy(&pending).trim_end_matches('\r').to_owned();
                data.push(text, stderr);
                pending.clear();
            }
            if byte != b'\n' { pending.push(byte); }
        }
        drop(data);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    if !pending.is_empty() {
        buffer.lock().unwrap().push(String::from_utf8_lossy(&pending).into_owned(), stderr);
        ctx.request_repaint();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drawer_renders_and_drop_cancels_stream() {
        let ctx = egui::Context::default();
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        buffer.lock().unwrap().push("2026-11-05T12:00:00Z server listening on :8080".into(), false);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut logs = ContainerLogs { id: "a".repeat(64), name: "shop-web-1".into(), buffer, cancel: cancel.clone(), follow: true };
        for _ in 0..3 {
            let output = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(760.0, 450.0))), ..Default::default() }, |ctx| {
                assert!(!logs.show(ctx));
                egui::CentralPanel::default().show(ctx, |ui| { ui.label("Container table"); });
            });
            assert!(!output.shapes.is_empty());
        }
        drop(logs);
        assert!(cancel.load(Ordering::Relaxed));
    }
    #[cfg(unix)]
    #[test]
    fn follows_both_streams_and_cancels_child() {
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut command = Command::new("sh");
        command.args(["-c", "printf 'stdout line\\n'; printf 'stderr line\\n' >&2; exec sleep 30"])
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        let worker_buffer = buffer.clone();
        let worker_cancel = cancel.clone();
        let started = std::time::Instant::now();
        let worker = std::thread::spawn(move || stream_command(command, &worker_buffer, &worker_cancel, &egui::Context::default()));
        while buffer.lock().unwrap().lines.len() < 2 {
            if started.elapsed() > Duration::from_secs(5) { cancel.store(true, Ordering::Relaxed); panic!("log readers did not produce output"); }
            std::thread::sleep(Duration::from_millis(10));
        }
        cancel.store(true, Ordering::Relaxed);
        worker.join().unwrap().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        let data = buffer.lock().unwrap();
        assert!(data.lines.iter().any(|(text, stderr)| text == "stdout line" && !stderr));
        assert!(data.lines.iter().any(|(text, stderr)| text == "stderr line" && *stderr));
    }
    #[test]
    fn bounds_history_and_long_lines_and_preserves_stderr() {
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let text = format!("{}\n{}tail", "line\n".repeat(MAX_LINES + 20), "x".repeat(MAX_LINE_BYTES + 1));
        read_output(text.as_bytes(), true, &buffer, &egui::Context::default()).unwrap();
        let data = buffer.lock().unwrap();
        assert_eq!(data.lines.len(), MAX_LINES);
        assert!(data.lines.iter().all(|(line, stderr)| line.len() <= MAX_LINE_BYTES && *stderr));
        assert_eq!(data.lines.back().unwrap().0, "xtail");
    }
}
