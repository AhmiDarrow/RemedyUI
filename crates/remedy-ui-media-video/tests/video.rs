//! Video surfaces on a real view, drawn by the CPU renderer: frames from another thread, the
//! newest winning, fitting, closing, and what screen readers get.

use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Size};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_media_video::*;
use remedy_ui_render_cpu::{CpuCompositor, Frame};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

struct Screen {
    ui: Ui,
    comp: Compositor,
    cpu: CpuCompositor,
    t: u64,
    last: Option<Frame>,
}

impl Screen {
    fn new(ui: Ui) -> Self {
        Screen {
            ui,
            comp: Compositor::new(200, 100, 1.0, ScrollPhysics::CLAMPED),
            cpu: CpuCompositor::new(),
            t: 0,
            last: None,
        }
    }

    /// Runs posted work (frames arriving), then a frame.
    fn frame(&mut self) {
        let _ = remedy_ui_reactive::run_posted();
        let f = self.ui.frame(Size::new(200.0, 100.0), 1.0);
        if let Some(l) = f.layers {
            self.comp.commit(l);
        }
        self.t += 16;
        let plan = self.comp.frame(Duration::from_millis(self.t));
        if let Ok(frame) = self.cpu.present(&plan, 200, 100, Color::WHITE) {
            self.last = Some(frame);
        }
    }

    fn px(&self, x: u32, y: u32) -> [u8; 4] {
        self.last.as_ref().and_then(|f| f.pixel(x, y)).unwrap_or([0; 4])
    }
}

/// A solid `w` × `h` NV12 picture of video-range Y, U, V.
fn solid(w: u32, h: u32, y: u8, u: u8, v: u8) -> Picture {
    let mut data = vec![y; (w * h) as usize];
    for _ in 0..(w.div_ceil(2) * h.div_ceil(2)) {
        data.extend([u, v]);
    }
    Picture::packed(w, h, PixelFormat::Nv12, data)
}

fn near(a: [u8; 4], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

fn surface(fit: VideoFit) -> (Screen, El, VideoSink) {
    let ui = ui();
    let (el, sink) = video_surface(&ui, "Alex's screen", fit);
    ui.root().child(ui.el("col").column().width(200.0).height(100.0).child(el.clone()));
    let mut s = Screen::new(ui);
    s.frame();
    (s, el, sink)
}

#[test]
fn frames_from_another_thread_show_in_their_colors() {
    let (mut s, _el, sink) = surface(VideoFit::Fill);
    let sender = sink.clone();
    // BT.709 video-range red.
    std::thread::spawn(move || assert!(sender.push(solid(64, 32, 63, 102, 240))))
        .join()
        .unwrap_or_else(|_| unreachable!("the sender"));
    s.frame();
    let p = s.px(100, 50);
    assert!(near(p, [255, 0, 0], 6), "{p:?}");
    assert_eq!(sink.stats(), {
        let mut v_ = VideoStats::default();
        v_.pushed = 1;
        v_.shown = 1;
        v_.dropped = 0;
        v_
    });
}

#[test]
fn the_newest_frame_wins_and_the_rest_count_as_dropped() {
    let (mut s, _el, sink) = surface(VideoFit::Fill);
    for y in [40, 80, 120, 160, 235] {
        assert!(sink.push(solid(16, 16, y, 128, 128)));
    }
    s.frame();
    assert!(near(s.px(100, 50), [255, 255, 255], 2), "the last frame shows: {:?}", s.px(100, 50));
    assert_eq!(sink.stats(), {
        let mut v_ = VideoStats::default();
        v_.pushed = 5;
        v_.shown = 1;
        v_.dropped = 4;
        v_
    });
    // A repaint with no new frame shows nothing new.
    s.frame();
    assert_eq!(sink.stats().shown, 1);
}

#[test]
fn contain_letterboxes_and_cover_crops() {
    // A wide 4:1 picture in a 2:1 place.
    let (mut s, _el, sink) = surface(VideoFit::Contain);
    sink.push(solid(64, 16, 235, 128, 128));
    s.frame();
    assert!(near(s.px(100, 50), [255, 255, 255], 2), "the picture in the middle");
    assert!(near(s.px(100, 5), [0, 0, 0], 2), "black bars above: {:?}", s.px(100, 5));
    let (mut s, _el, sink) = surface(VideoFit::Cover);
    sink.push(solid(64, 16, 235, 128, 128));
    s.frame();
    assert!(near(s.px(100, 5), [255, 255, 255], 2), "covered to the edge");
    assert!(near(s.px(2, 50), [255, 255, 255], 2), "cropped at the sides, not shrunk");
}

#[test]
fn malformed_pictures_are_refused_and_a_closed_surface_says_so() {
    let (mut s, el, sink) = surface(VideoFit::Fill);
    let mut bad = solid(16, 16, 100, 128, 128);
    bad.data = vec![0u8; 10].into();
    assert!(!sink.push(bad));
    assert_eq!(sink.stats().dropped, 1);
    // Screen readers get an image with the label.
    let sem = el.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("semantics"));
    assert_eq!(sem.role(), remedy_ui_a11y::Role::Image);
    assert_eq!(sem.name(), Some("Alex's screen"));
    el.remove();
    s.frame();
    assert!(!sink.is_open());
    assert!(!sink.push(solid(16, 16, 100, 128, 128)), "nobody is watching");
}
