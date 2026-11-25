use std::{
    f32::consts::TAU,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, Frame, Margin, Pos2, Vec2, epaint::Mesh};
use windows_sys::Win32::{
    Foundation::RECT,
    UI::WindowsAndMessaging::{
        FindWindowW, GWL_EXSTYLE, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
        HWND_TOPMOST, LWA_COLORKEY, SPI_GETCLIENTAREAANIMATION, SPI_GETWORKAREA, SW_HIDE,
        SW_SHOWNOACTIVATE, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowLongW, SetWindowPos, ShowWindowAsync,
        SystemParametersInfoW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    },
};

use crate::{speech::Update, ui::emphasis};

const TITLE: &str = "Wiki Listening";
const TRANSCRIPT_DURATION: Duration = Duration::from_secs(3);

#[derive(Default)]
enum Display {
    #[default]
    Hidden,
    Listening(Instant),
    Heard {
        transcript: String,
        until: Instant,
        phase: f32,
    },
}

#[derive(Default)]
struct State {
    session: u64,
    display: Display,
}

#[derive(Clone)]
pub(crate) struct ListeningPanel {
    state: Arc<Mutex<State>>,
    animate: bool,
}

impl ListeningPanel {
    pub(crate) fn new() -> Self {
        let mut animate = 1i32;
        unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut animate as *mut i32).cast(),
                0,
            );
        }
        Self {
            state: Arc::default(),
            animate: animate != 0,
        }
    }

    pub(crate) fn reset(&self) -> u64 {
        let mut state = self.state.lock().expect("listening panel lock poisoned");
        state.session += 1;
        state.display = Display::Hidden;
        set_visible(false);
        state.session
    }

    pub(crate) fn update(&self, session: u64, update: &Update, ctx: &egui::Context) {
        let mut state = self.state.lock().expect("listening panel lock poisoned");
        if session != state.session {
            return;
        }
        let visibility = match update {
            Update::Awake => {
                state.display = Display::Listening(Instant::now());
                Some(true)
            }
            Update::Outcome { transcript, .. } => {
                let phase = match state.display {
                    Display::Listening(started) if self.animate => started.elapsed().as_secs_f32(),
                    _ => 0.,
                };
                state.display = Display::Heard {
                    transcript: transcript.clone(),
                    until: Instant::now() + TRANSCRIPT_DURATION,
                    phase,
                };
                None
            }
            Update::Waiting | Update::Error(_) => {
                state.display = Display::Hidden;
                Some(false)
            }
        };
        drop(state);
        if let Some(visible) = visibility {
            set_visible(visible);
        }
        if visibility != Some(false) {
            ctx.request_repaint_of(egui::ViewportId::from_hash_of(TITLE));
        }
    }

    pub(crate) fn register(&self, ctx: &egui::Context) {
        let panel = self.clone();
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of(TITLE),
            egui::ViewportBuilder::default()
                .with_title(TITLE)
                .with_inner_size(Vec2::new(540., 245.))
                .with_decorations(false)
                .with_resizable(false)
                .with_active(false)
                .with_taskbar(false)
                .with_mouse_passthrough(true)
                .with_always_on_top()
                .with_transparent(true)
                .with_visible(false),
            move |ctx, _| panel.paint(ctx),
        );
    }

    fn paint(&self, ctx: &egui::Context) {
        let mut state = self.state.lock().expect("listening panel lock poisoned");
        let (transcript, phase, repaint_after) = match &state.display {
            Display::Hidden => {
                set_visible(false);
                return;
            }
            Display::Listening(started) => {
                if self.animate {
                    (
                        None,
                        started.elapsed().as_secs_f32(),
                        Some(Duration::from_millis(16)),
                    )
                } else {
                    (None, 0., None)
                }
            }
            Display::Heard {
                transcript,
                until,
                phase,
            } => {
                let Some(remaining) = until.checked_duration_since(Instant::now()) else {
                    state.display = Display::Hidden;
                    set_visible(false);
                    return;
                };
                (Some(transcript.clone()), *phase, Some(remaining))
            }
        };
        drop(state);
        if let Some(delay) = repaint_after {
            ctx.request_repaint_after(delay);
        }

        egui::CentralPanel::default()
            .frame(Frame::NONE.inner_margin(Margin::symmetric(16, 0)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let (orb, response) =
                        ui.allocate_exact_size(Vec2::splat(138.), egui::Sense::hover());
                    ui.painter().add(orb_mesh(orb.center(), phase));
                    if let Some(text) = transcript {
                        draw_transcript(ui, &text);
                    } else {
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Listening")
                        });
                    }
                });
            });
    }
}

fn orb_mesh(center: Pos2, time: f32) -> Mesh {
    const SEGMENTS: u32 = 96;
    const SURFACE_RINGS: u32 = 26;
    let mut mesh = Mesh::default();
    let tilt = 0.32 * (time * 1.1).sin() - 0.23;
    let (sin_tilt, cos_tilt) = tilt.sin_cos();
    let radius = 53. + 1.4 * (time * 2.3).sin();

    for ring in 0..=SURFACE_RINGS + 1 {
        let surface = ring <= SURFACE_RINGS;
        let r = if surface {
            ring as f32 / SURFACE_RINGS as f32
        } else {
            1.015
        };
        let depth = (1. - r * r).max(0.).sqrt();
        for segment in 0..SEGMENTS {
            let angle = segment as f32 * TAU / SEGMENTS as f32;
            let (sin, cos) = angle.sin_cos();
            let x = r * cos;
            let y = r * sin;
            let color = if surface {
                let across = x * cos_tilt + y * sin_tilt;
                let along = y * cos_tilt - x * sin_tilt;
                let fold = across * 6.5 + 2.1 * (along * 2.4 - time * 0.7).sin() + 1.25 * depth
                    - time * 1.8;
                let light = ((0.5 + 0.5 * fold.sin() - 0.23) / 0.66).clamp(0., 1.);
                let light = light * light * (3. - 2. * light);
                let violet = 0.5 + 0.5 * (along * 3. + across * 1.2 + time).sin();
                let rim = (1. - depth).powi(4);
                Color32::from_rgb(
                    (11. + 33. * rim + light * (182. + 47. * violet)) as u8,
                    (9. + 24. * rim + light * (215. - 42. * violet)) as u8,
                    (52. + 119. * rim + light * (200. - 119. * rim)) as u8,
                )
            } else {
                Color32::TRANSPARENT
            };
            mesh.colored_vertex(center + Vec2::new(x, y) * radius, color);
            if ring > 0 {
                let a = (ring - 1) * SEGMENTS + segment;
                let b = (ring - 1) * SEGMENTS + (segment + 1) % SEGMENTS;
                mesh.add_triangle(a, a + SEGMENTS, b);
                mesh.add_triangle(b, a + SEGMENTS, b + SEGMENTS);
            }
        }
    }
    mesh
}

fn draw_transcript(ui: &mut egui::Ui, text: &str) {
    let (pos, galley, response) = egui::Label::new(emphasis(text, 22.).color(Color32::WHITE))
        .wrap()
        .selectable(false)
        .layout_in_ui(ui);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    for offset in [
        Vec2::new(-1., -1.),
        Vec2::new(1., -1.),
        Vec2::new(-1., 1.),
        Vec2::new(1., 1.),
    ] {
        ui.painter().galley_with_override_text_color(
            pos + offset,
            galley.clone(),
            Color32::from_rgb(12, 11, 26),
        );
    }
    ui.painter().galley(pos, galley, Color32::WHITE);
}

// A hidden eframe root cannot repaint to open a viewport from the tray.
fn set_visible(visible: bool) {
    let title: Vec<_> = TITLE.encode_utf16().chain([0]).collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    let mut pid = 0;
    if hwnd.is_null()
        || unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } == 0
        || pid != std::process::id()
    {
        return;
    }
    unsafe {
        if !visible {
            ShowWindowAsync(hwnd, SW_HIDE);
            return;
        }
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let popup_style = (style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW;
        if style != popup_style {
            SetWindowLongW(hwnd, GWL_EXSTYLE, popup_style as i32);
        }
        // The click-through Windows layer keys out the renderer's clear color.
        SetLayeredWindowAttributes(hwnd, 0, 0, LWA_COLORKEY);
        let mut work = RECT::default();
        let mut bounds = RECT::default();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) != 0
            && GetWindowRect(hwnd, &mut bounds) != 0
        {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                (work.left + work.right - (bounds.right - bounds.left)) / 2,
                work.bottom - (bounds.bottom - bounds.top) - 18,
                0,
                0,
                SWP_ASYNCWINDOWPOS
                    | SWP_FRAMECHANGED
                    | SWP_NOACTIVATE
                    | SWP_NOSIZE
                    | SWP_SHOWWINDOW,
            );
        } else {
            ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orb_mesh_changes_with_animation_phase() {
        let still = orb_mesh(Pos2::ZERO, 0.);
        let moving = orb_mesh(Pos2::ZERO, 0.5);
        assert!(still.is_valid() && moving.is_valid());
        assert_ne!(still.vertices, moving.vertices);
    }
}
