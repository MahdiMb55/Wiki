use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result};
use eframe::egui::{
    self, Align, Color32, FontData, FontFamily, FontId, Frame, Id, Layout, Margin, RichText,
    Stroke, Ui, UiBuilder, Vec2, ViewportCommand,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use wiki::commands::{Action, Command, Commands};
use windows_sys::Win32::{
    Foundation::HWND,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        FindWindowW, ICON_BIG, ICON_SMALL, IMAGE_ICON, IsIconic, LR_DEFAULTSIZE, LR_SHARED,
        LoadImageW, PostMessageW, SW_HIDE, SW_RESTORE, SW_SHOW, SendMessageW, SetForegroundWindow,
        ShowWindow, WM_CLOSE, WM_SETICON,
    },
};

use crate::{
    actions,
    listening::ListeningPanel,
    speech::{Speech, Update},
};

const INDIGO: Color32 = Color32::from_rgb(42, 37, 143);
const PAPER: Color32 = Color32::from_rgb(250, 249, 248);
const BAND: Color32 = Color32::from_rgb(233, 230, 242);
const HEADER: Color32 = Color32::from_rgb(235, 233, 232);
const LINE: Color32 = Color32::from_rgb(211, 207, 208);
const INK: Color32 = Color32::from_rgb(25, 24, 28);
const MUTED: Color32 = Color32::from_rgb(88, 85, 93);
const ERROR: Color32 = Color32::from_rgb(147, 34, 47);
const APP_ICON_RESOURCE: u16 = 1;
const SMALL_ICON_RESOURCE: u16 = 2;

pub struct Wiki {
    commands: Arc<RwLock<Commands>>,
    config_file: Option<PathBuf>,
    error: Option<String>,
    speech: Option<Speech>,
    status: Status,
    activity: Option<String>,
    listening: ListeningPanel,
    editor: Option<Editor>,
    hwnd: isize,
    tray: TrayIcon,
    logo: egui::TextureHandle,
    quit: Arc<AtomicBool>,
    start_hidden: bool,
}

enum Status {
    Loading,
    Waiting,
    Awake,
    Paused,
    Error,
}

impl Status {
    fn label(&self) -> &'static str {
        match self {
            Self::Loading => "Starting microphone…",
            Self::Waiting => "Waiting for “Hey Wiki”",
            Self::Awake => "Listening for a command…",
            Self::Paused => "Listening paused",
            Self::Error => "Microphone unavailable",
        }
    }
}

impl Wiki {
    pub fn new(cc: &eframe::CreationContext<'_>, start_hidden: bool) -> Result<Self> {
        install_style(&cc.egui_ctx);
        let RawWindowHandle::Win32(hwnd) = cc.window_handle()?.as_raw() else {
            anyhow::bail!("Wiki needs a Windows desktop window.");
        };
        let hwnd = hwnd.hwnd.get();
        set_window_icons(hwnd as HWND)?;
        let logo = load_logo(&cc.egui_ctx)?;
        let quit = Arc::new(AtomicBool::new(false));
        let tray = make_tray(hwnd, quit.clone(), &cc.egui_ctx)?;
        cc.egui_ctx
            .send_viewport_cmd(ViewportCommand::SetTheme(egui::SystemTheme::Light));
        let (commands, config_file, error) = match actions::config_file() {
            Ok(path) => match Commands::load(&path) {
                Ok(commands) => (commands, Some(path), None),
                Err(err) => (Commands::empty(), None, Some(format!("{err:#}"))),
            },
            Err(err) => (Commands::empty(), None, Some(format!("{err:#}"))),
        };
        let mut wiki = Self {
            commands: Arc::new(RwLock::new(commands)),
            config_file,
            error,
            speech: None,
            status: Status::Paused,
            activity: None,
            listening: ListeningPanel::new(),
            editor: None,
            hwnd,
            tray,
            logo,
            quit,
            start_hidden,
        };
        wiki.toggle_listening(&cc.egui_ctx);
        Ok(wiki)
    }

    fn toggle_listening(&mut self, ctx: &egui::Context) {
        self.activity = None;
        let session = self.listening.reset();
        if self.speech.take().is_some() {
            self.status = Status::Paused;
        } else {
            let ctx = ctx.clone();
            let listening = self.listening.clone();
            match Speech::start(self.commands.clone(), move |update| {
                listening.update(session, update, &ctx);
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }) {
                Ok(speech) => {
                    self.speech = Some(speech);
                    self.status = Status::Loading;
                }
                Err(err) => {
                    self.status = Status::Error;
                    self.activity = Some(format!("Could not start listening: {err}"));
                }
            }
        }
        self.update_tooltip();
    }

    fn update_tooltip(&self) {
        let _ = self
            .tray
            .set_tooltip(Some(format!("Wiki · {}", self.status.label())));
    }

    fn read_speech(&mut self) {
        while let Some(update) = self
            .speech
            .as_ref()
            .and_then(|speech| speech.updates.try_recv().ok())
        {
            match update {
                Update::Waiting => self.status = Status::Waiting,
                Update::Awake => self.status = Status::Awake,
                Update::Outcome { message, .. } => {
                    self.status = Status::Waiting;
                    self.activity = Some(message);
                }
                Update::Error(error) => {
                    self.speech = None;
                    self.status = Status::Error;
                    self.activity = Some(error);
                }
            }
            self.update_tooltip();
        }
    }

    fn save_editor(&self, editor: &Editor, delete: bool) -> Result<()> {
        let path = self
            .config_file
            .as_ref()
            .context("The command file is unavailable.")?;
        let mut commands = self.commands.read().expect("command lock poisoned").clone();
        if delete {
            commands.remove(editor.index.context("Select a command to delete.")?);
        } else {
            if let Action::OpenApp(path) = &editor.command.action {
                anyhow::ensure!(
                    std::path::Path::new(path).is_file(),
                    "Choose an existing app's .exe file."
                );
            }
            commands.set(editor.index, editor.command.clone())?;
        }
        commands.save(path).context("Could not save commands.")?;
        *self.commands.write().expect("command lock poisoned") = commands;
        Ok(())
    }
}

impl eframe::App for Wiki {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.start_hidden {
            self.start_hidden = false;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.quit.load(Ordering::Acquire) {
                return;
            }
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            unsafe { ShowWindow(self.hwnd as HWND, SW_HIDE) };
        }
        self.read_speech();

        egui::TopBottomPanel::top("listening_bar")
            .frame(
                Frame::new()
                    .fill(BAND)
                    .inner_margin(Margin::symmetric(25, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(42.);
                    ui.add(egui::Image::new((self.logo.id(), Vec2::splat(40.))).alt_text("Wiki"));
                    ui.add_space(6.);
                    ui.label(emphasis(self.status.label(), 22.).color(INK));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if self.speech.is_some() {
                            "Pause listening"
                        } else if matches!(self.status, Status::Error) {
                            "Retry microphone"
                        } else {
                            "Resume listening"
                        };
                        if text_button(ui, label, 18.).clicked() {
                            self.toggle_listening(ctx);
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(PAPER)
                    .inner_margin(Margin::symmetric(25, 20)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(emphasis("Commands", 46.).color(INK));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let add = primary("+  Add command", 18.);
                        if ui.add_enabled(self.config_file.is_some(), add).clicked() {
                            self.editor = Some(Editor::new(None, None));
                        }
                    });
                });
                ui.label(
                    RichText::new("Add a phrase and choose what Wiki should do.")
                        .size(20.)
                        .color(MUTED),
                );
                ui.add_space(15.);
                if let Some(error) = &self.error {
                    ui.label(RichText::new(error).color(ERROR));
                    ui.add_space(8.);
                }
                if let Some(activity) = &self.activity {
                    let color = if matches!(self.status, Status::Error) {
                        ERROR
                    } else {
                        MUTED
                    };
                    ui.label(RichText::new(activity).color(color));
                    ui.add_space(8.);
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.;
                        row(ui, "Say", "Action", true, None);
                        let commands = self.commands.read().expect("command lock poisoned");
                        for (index, command) in commands.items().iter().enumerate() {
                            let phrase = if command.action == Action::SearchChrome {
                                format!("{} …", command.phrase)
                            } else {
                                command.phrase.clone()
                            };
                            if row(ui, &phrase, command.action.label(), false, Some(index)) {
                                self.editor = Some(Editor::new(Some(index), Some(command)));
                            }
                            ui.separator();
                        }
                        if commands.items().is_empty() && self.error.is_none() {
                            ui.add_space(14.);
                            ui.label("Add a command to choose what Wiki can do.");
                        }
                    });
            });

        if let Some(mut editor) = self.editor.take() {
            match editor.show(ctx) {
                Form::Cancel => {}
                Form::Open => self.editor = Some(editor),
                result => {
                    if let Err(err) = self.save_editor(&editor, result == Form::Delete) {
                        editor.error = Some(format!("{err:#}"));
                        self.editor = Some(editor);
                    }
                }
            }
        }
        self.listening.register(ctx);
    }
}

fn primary(label: &str, size: f32) -> egui::Button<'_> {
    egui::Button::new(emphasis(label, size).color(Color32::WHITE))
        .fill(INDIGO)
        .min_size(Vec2::new(132., 42.))
}

pub(crate) fn emphasis(text: &str, size: f32) -> RichText {
    RichText::new(text).font(FontId::new(size, FontFamily::Name("emphasis".into())))
}

fn text_button(ui: &mut Ui, label: &str, size: f32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(size).color(INDIGO)).frame_when_inactive(false),
    )
}

fn row(ui: &mut Ui, phrase: &str, action: &str, heading: bool, index: Option<usize>) -> bool {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(width, if heading { 48. } else { 53. }),
        egui::Sense::hover(),
    );
    if heading {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius {
                nw: 6,
                ne: 6,
                ..Default::default()
            },
            HEADER,
        );
    }
    let mut edit = false;
    let mut column = |start: f32, end: f32, text: &str| {
        let bounds = egui::Rect::from_min_max(
            egui::pos2(rect.left() + start, rect.top()),
            egui::pos2(rect.left() + end, rect.bottom()),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(bounds)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                let text_style = if heading {
                    emphasis(text, 20.)
                } else {
                    RichText::new(text).size(20.)
                };
                ui.add(egui::Label::new(text_style.color(INK)).truncate())
                    .on_hover_text(text);
            },
        );
    };
    column(20., width * 0.45, phrase);
    column(width * 0.45 + 10., width - 87., action);
    if let Some(index) = index {
        ui.push_id(index, |ui| {
            let bounds = rect
                .with_min_x(rect.right() - 77.)
                .with_max_x(rect.right() - 15.);
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(bounds)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    let button = text_button(ui, "Edit", 18.);
                    button.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("Edit {phrase}"),
                        )
                    });
                    edit = button.clicked();
                },
            );
        });
    }
    edit
}

struct Editor {
    command: Command,
    index: Option<usize>,
    error: Option<String>,
    focus_phrase: bool,
    confirm_delete: bool,
}

#[derive(PartialEq, Eq)]
enum Form {
    Open,
    Save,
    Cancel,
    Delete,
}

impl Editor {
    fn new(index: Option<usize>, command: Option<&Command>) -> Self {
        Self {
            command: command.cloned().unwrap_or_else(|| Command {
                phrase: String::new(),
                action: Action::OpenApp(String::new()),
            }),
            index,
            error: None,
            focus_phrase: true,
            confirm_delete: false,
        }
    }

    fn show(&mut self, ctx: &egui::Context) -> Form {
        let mut result = Form::Open;
        let modal = egui::Modal::new(Id::new("command_editor")).show(ctx, |ui| {
            ui.set_width(440.);
            ui.heading(if self.index.is_some() {
                "Edit command"
            } else {
                "Add command"
            });
            ui.add_space(10.);
            let label = ui.label("When I say");
            let phrase = ui
                .add(
                    egui::TextEdit::singleline(&mut self.command.phrase)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label.id);
            if std::mem::take(&mut self.focus_phrase) {
                phrase.request_focus();
            }
            ui.add_space(8.);
            let label = ui.label("Do this");
            egui::ComboBox::from_id_salt("action")
                .width(220.)
                .selected_text(self.command.action.label())
                .show_ui(ui, |ui| {
                    for action in [
                        Action::OpenApp(String::new()),
                        Action::OpenWebsite(String::new()),
                        Action::MediaToggle,
                        Action::SwitchApps,
                        Action::SearchChrome,
                    ] {
                        if ui
                            .selectable_label(
                                action.label() == self.command.action.label(),
                                action.label(),
                            )
                            .clicked()
                            && action.label() != self.command.action.label()
                        {
                            self.command.action = action;
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            match &mut self.command.action {
                Action::OpenChrome => {
                    ui.horizontal(|ui| {
                        ui.label("Google Chrome");
                        if ui.button("Choose another app…").clicked()
                            && let Some(path) = choose_exe()
                        {
                            self.command.action = Action::OpenApp(path);
                        }
                    });
                }
                Action::OpenApp(target) => {
                    let label = ui.label("App file");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(target).desired_width(340.))
                            .labelled_by(label.id);
                        if ui.button("Browse…").clicked()
                            && let Some(path) = choose_exe()
                        {
                            *target = path;
                        }
                    });
                }
                Action::OpenWebsite(target) => {
                    let label = ui.label("Website URL");
                    ui.add(
                        egui::TextEdit::singleline(target)
                            .desired_width(f32::INFINITY)
                            .hint_text("https://example.com"),
                    )
                    .labelled_by(label.id);
                }
                Action::SearchChrome => {
                    ui.label("Say this phrase followed by what you want to search for.");
                }
                Action::MediaToggle => {
                    ui.label("Sends the Windows play/pause media key.");
                }
                Action::SwitchApps => {
                    ui.label("Sends Alt+Tab to switch to the previous app.");
                }
            }
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).color(ERROR));
            }
            ui.add_space(10.);
            ui.horizontal(|ui| {
                if ui.add(primary("Save command", 16.)).clicked() {
                    result = Form::Save;
                }
                if ui.button("Cancel").clicked() {
                    result = Form::Cancel;
                }
                if self.index.is_some()
                    && ui
                        .button(if self.confirm_delete {
                            "Confirm delete"
                        } else {
                            "Delete"
                        })
                        .clicked()
                {
                    if self.confirm_delete {
                        result = Form::Delete;
                    } else {
                        self.confirm_delete = true;
                    }
                }
            });
        });
        if modal.should_close() {
            Form::Cancel
        } else {
            result
        }
    }
}

fn choose_exe() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Windows apps", &["exe"])
        .pick_file()
        .and_then(|path| path.into_os_string().into_string().ok())
}

fn install_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let windows =
        std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:/Windows"), PathBuf::from);
    if let Ok(bytes) = std::fs::read(windows.join("Fonts/segoeui.ttf")) {
        fonts
            .font_data
            .insert("Segoe UI".into(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "Segoe UI".into());
    }
    let mut emphasis = fonts.families[&FontFamily::Proportional].clone();
    if let Ok(bytes) = std::fs::read(windows.join("Fonts/segoeuib.ttf")) {
        fonts.font_data.insert(
            "Segoe UI Bold".into(),
            Arc::new(FontData::from_owned(bytes)),
        );
        emphasis.insert(0, "Segoe UI Bold".into());
    }
    fonts
        .families
        .insert(FontFamily::Name("emphasis".into()), emphasis);
    ctx.set_fonts(fonts);
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = PAPER;
    visuals.window_fill = PAPER;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    visuals.selection.stroke = Stroke::new(1.5_f32, INDIGO);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.button_padding = egui::vec2(10., 7.);
        style.spacing.item_spacing = egui::vec2(10., 8.);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(16.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(16.));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, FontId::proportional(27.));
    });
}

fn load_logo(ctx: &egui::Context) -> Result<egui::TextureHandle> {
    let decoder = png::Decoder::new(Cursor::new(include_bytes!("../assets/Wiki the boy.png")));
    let mut reader = decoder.read_info()?;
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba)?;
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [info.width as usize, info.height as usize],
        &rgba[..info.buffer_size()],
    );
    Ok(ctx.load_texture("Wiki logo", image, egui::TextureOptions::LINEAR))
}

fn set_window_icons(hwnd: HWND) -> Result<()> {
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    for (kind, resource) in [
        (ICON_BIG, APP_ICON_RESOURCE),
        (ICON_SMALL, SMALL_ICON_RESOURCE),
    ] {
        let icon = unsafe {
            LoadImageW(
                module,
                resource as usize as *const u16,
                IMAGE_ICON,
                0,
                0,
                LR_DEFAULTSIZE | LR_SHARED,
            )
        };
        if icon.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("Could not load Wiki's window icon");
        }
        unsafe { SendMessageW(hwnd, WM_SETICON, kind as usize, icon as isize) };
    }
    Ok(())
}

fn make_tray(hwnd: isize, quit: Arc<AtomicBool>, ctx: &egui::Context) -> Result<TrayIcon> {
    let menu = Menu::new();
    let open = MenuItem::new("Open Wiki", true, None);
    let exit = MenuItem::new("Quit Wiki", true, None);
    menu.append_items(&[&open, &exit])?;
    let open = open.id().clone();
    let exit = exit.id().clone();
    let repaint = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == open {
            restore(hwnd);
            repaint.request_repaint();
        }
        if event.id == exit {
            quit.store(true, Ordering::Release);
            restore(hwnd);
            unsafe { PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0) };
            repaint.request_repaint();
        }
    }));
    let repaint = ctx.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            restore(hwnd);
            repaint.request_repaint();
        }
    }));
    Ok(TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(Icon::from_resource(SMALL_ICON_RESOURCE, None)?)
        .with_tooltip("Wiki")
        .build()?)
}

pub fn reopen() {
    let title: Vec<_> = "Wiki".encode_utf16().chain([0]).collect();
    let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if !window.is_null() {
        restore(window as isize);
    }
}

fn restore(hwnd: isize) {
    let window = hwnd as HWND;
    unsafe {
        ShowWindow(
            window,
            if IsIconic(window) != 0 {
                SW_RESTORE
            } else {
                SW_SHOW
            },
        );
        SetForegroundWindow(window);
    }
}
