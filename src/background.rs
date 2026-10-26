use eframe::egui;

#[derive(Clone, Copy)]
pub enum Action {
    Open,
    Widget,
    Hide,
    Quit,
}

pub struct Background {
    pub compact: bool,
    pub widget_visible: bool,
    pub quitting: bool,
    main_rect: Option<egui::Rect>,
    main_maximized: bool,
    widget_position: Option<egui::Pos2>,
    #[cfg(windows)]
    tray: Option<tray_icon::TrayIcon>,
    #[cfg(windows)]
    events: std::sync::mpsc::Receiver<Action>,
    #[cfg(windows)]
    hwnd: isize,
    #[cfg(windows)]
    rounded_region: Option<(i32, i32)>,
    pub error: Option<String>,
}

impl Background {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        #[cfg(windows)]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match cc.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
                _ => 0,
            }
        };
        #[cfg(windows)]
        let (tray, events, error) = {
            use tray_icon::{
                TrayIconBuilder, TrayIconEvent,
                menu::{Menu, MenuEvent, MenuItem},
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let menu = Menu::new();
            let open = MenuItem::new("Open VijiDeck", true, None);
            let widget = MenuItem::new("Show widget", true, None);
            let hide = MenuItem::new("Hide widget", true, None);
            let quit = MenuItem::new("Quit VijiDeck", true, None);
            let result = (|| -> Result<_, Box<dyn std::error::Error>> {
                menu.append_items(&[&open, &widget, &hide, &quit])?;
                let image = image::load_from_memory(include_bytes!(
                    "../Glossy Donut Chart in Blue Tray.png"
                ))?
                .to_rgba8();
                let image = image::imageops::resize(
                    &image,
                    32,
                    32,
                    image::imageops::FilterType::Lanczos3,
                );
                let (width, height) = image.dimensions();
                Ok(TrayIconBuilder::new()
                    .with_tooltip("VijiDeck — system monitor")
                    .with_icon(tray_icon::Icon::from_rgba(image.into_raw(), width, height)?)
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .build()?)
            })();
            let repaint = ctx.clone();
            let menu_tx = tx.clone();
            let open = open.id().clone();
            let widget = widget.id().clone();
            let hide = hide.id().clone();
            let quit = quit.id().clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let action = if event.id == open {
                    Action::Open
                } else if event.id == widget {
                    Action::Widget
                } else if event.id == hide {
                    Action::Hide
                } else if event.id == quit {
                    Action::Quit
                } else {
                    return;
                };
                menu_tx.send(action).ok();
                wake_window(hwnd);
                repaint.request_repaint();
            }));
            let repaint = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: tray_icon::MouseButton::Left,
                        button_state: tray_icon::MouseButtonState::Up,
                        ..
                    }
                ) {
                    tx.send(Action::Open).ok();
                    wake_window(hwnd);
                    repaint.request_repaint();
                }
            }));
            match result {
                Ok(tray) => (Some(tray), rx, None),
                Err(error) => (None, rx, Some(format!("Tray unavailable: {error}"))),
            }
        };
        #[cfg(not(windows))]
        let _ = ctx;
        Self {
            compact: false,
            widget_visible: false,
            quitting: false,
            main_rect: None,
            main_maximized: false,
            widget_position: None,
            #[cfg(windows)]
            tray,
            #[cfg(windows)]
            events,
            #[cfg(windows)]
            hwnd,
            #[cfg(windows)]
            rounded_region: None,
            #[cfg(windows)]
            error,
            #[cfg(not(windows))]
            error: None,
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        while let Ok(action) = self.events.try_recv() {
            self.apply(action, ctx);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let action = if !self.compact {
                Action::Widget
            } else if self.can_hide() {
                Action::Hide
            } else {
                Action::Open
            };
            self.apply(action, ctx);
        }
    }

    #[cfg(windows)]
    pub fn sync_window_shape(&mut self) {
        use windows_sys::Win32::{
            Foundation::RECT,
            Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn},
            UI::WindowsAndMessaging::GetClientRect,
        };
        if self.hwnd == 0 {
            return;
        }
        unsafe {
            if !self.compact {
                if self.rounded_region.take().is_some() {
                    SetWindowRgn(self.hwnd as _, std::ptr::null_mut(), 1);
                }
                return;
            }
            let mut rect = RECT::default();
            if GetClientRect(self.hwnd as _, &mut rect) == 0 {
                return;
            }
            let size = (rect.right, rect.bottom);
            if size.0 <= 0 || size.1 <= 0 || self.rounded_region == Some(size) {
                return;
            }
            // Clip the native window itself so the transparent corners are genuinely rounded.
            let region = CreateRoundRectRgn(0, 0, size.0 + 1, size.1 + 1, 32, 32);
            if !region.is_null() && SetWindowRgn(self.hwnd as _, region, 1) != 0 {
                self.rounded_region = Some(size);
            }
        }
    }

    #[cfg(not(windows))]
    pub fn sync_window_shape(&mut self) {}

    pub fn can_hide(&self) -> bool {
        #[cfg(windows)]
        {
            self.tray.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open => {
                let was_compact = self.compact;
                if was_compact && self.widget_visible {
                    self.widget_position = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));
                }
                self.compact = false;
                self.widget_visible = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                if was_compact {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                        900.0, 580.0,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                        egui::WindowLevel::Normal,
                    ));
                    if let Some(rect) = self.main_rect {
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(rect.min));
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(rect.size()));
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(self.main_maximized));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Action::Widget => {
                if !self.compact {
                    self.main_rect = ctx.input(|i| {
                        let viewport = i.viewport();
                        viewport.inner_rect.map(|inner| {
                            egui::Rect::from_min_size(
                                viewport.outer_rect.map_or(inner.min, |outer| outer.min),
                                inner.size(),
                            )
                        })
                    });
                    self.main_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                }
                self.compact = true;
                self.widget_visible = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(1.0, 1.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(300.0, 160.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::AlwaysOnTop,
                ));
                if let Some(position) = self.widget_position {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
                }
            }
            Action::Hide if self.can_hide() => {
                self.widget_position = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));
                self.widget_visible = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            Action::Hide => {}
            Action::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

#[cfg(windows)]
fn wake_window(hwnd: isize) {
    // Hidden Windows windows do not receive the redraw that drains egui commands.
    // Tray callbacks run on the native UI thread, so reveal it before requesting repaint.
    if hwnd != 0 {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd as _,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
            );
        }
    }
}
