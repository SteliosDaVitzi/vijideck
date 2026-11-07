#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod background;
mod cleanup;
mod cleanup_ui;
mod command;
mod docker;
mod docker_logs;
mod docker_resources;
mod docker_terminal;
mod environment;
mod environment_catalog;
mod environment_icons;
mod environment_origin;
mod environment_settings;
mod environment_variables;
mod gpu;
mod logs;
mod logs_access;
mod logs_list;
mod logs_ui;
mod overview;
mod ports;
mod process_control;
mod processes;
#[cfg(not(windows))]
mod profile;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod profile_metadata;
mod services;
mod services_ui;
mod storage;
mod ui;
mod volumes;
mod worker;

fn main() -> eframe::Result {
    let icon = image::load_from_memory(include_bytes!(
        "../packaging/linux/icons/hicolor/64x64/apps/vijideck.png"
    ))
    .expect("embedded application icon is a valid PNG")
    .to_rgba8();
    let (width, height) = icon.dimensions();
    eframe::run_native(
        "VijiDeck",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_transparent(true)
                .with_icon(std::sync::Arc::new(eframe::egui::IconData {
                    rgba: icon.into_raw(),
                    width,
                    height,
                }))
                .with_inner_size(initial_window_size())
                .with_min_inner_size([900.0, 580.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::VijiDeck::new(cc)))),
    )
}

fn initial_window_size() -> [f32; 2] {
    #[cfg(feature = "ui-preview")]
    if std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some()
        && std::env::var_os("VIJIDECK_PREVIEW_COMPACT").is_some()
    {
        return [900.0, 580.0];
    }
    [1320.0, 820.0]
}
