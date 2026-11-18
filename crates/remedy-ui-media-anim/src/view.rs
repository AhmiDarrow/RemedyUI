//! The playing of one element: which frame it shows, the timer for the next, and when it
//! stops (unseen, or motion reduced).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

use remedy_ui_media_image::{ImageState, Source};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{Ui, WeakUi};

use crate::stream::{Event, Next, Shown, Step, Stream};

/// What an element has to say about how it is shown, whichever stream it is on.
pub(crate) struct Control {
    /// The window, held weakly: a player never keeps a closed window alive.
    pub ui: WeakUi,
    /// Some of the element is on screen.
    pub visible: Cell<bool>,
    /// The system asks for reduced motion.
    pub reduced: Cell<bool>,
    /// Under reduced motion, the person pressed play.
    pub play: Cell<bool>,
    /// There is more than one frame (the play control shows when motion is reduced).
    pub has_motion: Signal<bool>,
    view: RefCell<Weak<View>>,
}

impl Control {
    pub fn new(ui: &Ui) -> Control {
        Control {
            ui: ui.downgrade(),
            visible: Cell::new(false),
            reduced: Cell::new(false),
            play: Cell::new(false),
            has_motion: Signal::new(false),
            view: RefCell::new(Weak::new()),
        }
    }

    /// Looks again at whether to play.
    pub fn refresh(&self) {
        let view = self.view.borrow().upgrade();
        if let Some(v) = view {
            v.refresh();
        }
    }

    fn active(&self) -> bool {
        self.visible.get() && (!self.reduced.get() || self.play.get())
    }
}

/// What a playing element is waiting for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    No,
    /// The frame the playhead is on is not decoded yet.
    Frame,
    /// The frame is up; whether another follows is not known yet.
    More,
    /// The frame's time is up and the next is not decoded yet.
    Advance,
}

/// Keeps a [`View`] going while the image element wants it.
pub(crate) struct Lease(Rc<View>);

impl Lease {
    /// Starts showing `source` in a box of `box_px` device pixels, reporting to `state`.
    pub fn new(
        ctl: &Rc<Control>,
        source: &Source,
        box_px: (u32, u32),
        cover: bool,
        state: Signal<ImageState>,
    ) -> Lease {
        let stream = Stream::request(source, box_px, cover);
        let view = Rc::new_cyclic(|me| View {
            ctl: ctl.clone(),
            source: source.clone(),
            box_px,
            cover,
            state,
            stream: RefCell::new(stream),
            token: Cell::new(0),
            me: me.clone(),
            timer: Cell::new(None),
            running: Cell::new(false),
            dead: Cell::new(false),
            cur: RefCell::new(None),
            poster_shown: Cell::new(false),
            wait: Cell::new(Wait::No),
        });
        view.listen();
        *ctl.view.borrow_mut() = Rc::downgrade(&view);
        view.refresh();
        Lease(view)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

pub(crate) struct View {
    ctl: Rc<Control>,
    source: Source,
    box_px: (u32, u32),
    cover: bool,
    state: Signal<ImageState>,
    stream: RefCell<Arc<Stream>>,
    token: Cell<u64>,
    me: Weak<View>,
    timer: Cell<Option<u64>>,
    /// Counted among the stream's players.
    running: Cell<bool>,
    dead: Cell<bool>,
    /// The frame shown, when one from the playhead is (not the poster).
    cur: RefCell<Option<Shown>>,
    poster_shown: Cell<bool>,
    wait: Cell<Wait>,
}

impl View {
    fn stream(&self) -> Arc<Stream> {
        self.stream.borrow().clone()
    }

    fn listen(&self) {
        let me = self.me.clone();
        let token = self.stream().listen(Rc::new(move |e| {
            if let Some(v) = me.upgrade() {
                v.on_event(e);
            }
        }));
        self.token.set(token);
    }

    fn on_event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Changed => self.refresh(),
            Event::Stale => self.reload(),
        }
    }

    /// The file changed: play the new one.
    fn reload(self: &Rc<Self>) {
        if self.dead.get() {
            return;
        }
        let old = self.stream();
        self.halt();
        old.unlisten(self.token.get());
        *self.stream.borrow_mut() = Stream::request(&self.source, self.box_px, self.cover);
        *self.cur.borrow_mut() = None;
        self.poster_shown.set(false);
        self.listen();
        self.refresh();
    }

    /// Looks at what is wanted and what has happened, and does what follows.
    pub fn refresh(self: &Rc<Self>) {
        if self.dead.get() {
            return;
        }
        let stream = self.stream();
        if let Some(e) = stream.failure() {
            self.halt();
            let _ = self.state.try_set(ImageState::Failed(e));
            return;
        }
        let Some(size) = stream.source_size() else { return };
        self.ctl.has_motion.set_if_changed(stream.is_animated());
        match (self.ctl.active(), self.running.get()) {
            (true, false) => self.start(&stream, size),
            (false, true) => {
                self.halt();
                // Motion reduced, or the person paused: back to the first frame.
                if self.ctl.reduced.get() {
                    self.show_poster(&stream, size);
                }
            }
            (true, true) => self.go_on(&stream, size),
            (false, false) => {
                if !self.poster_shown.get() && self.cur.borrow().is_none() {
                    self.show_poster(&stream, size);
                }
            }
        }
    }

    fn start(self: &Rc<Self>, stream: &Arc<Stream>, size: (u32, u32)) {
        self.running.set(true);
        // Pressing play under reduced motion starts from the first frame.
        stream.activate(self.ctl.reduced.get(), Instant::now());
        self.present(stream, size);
    }

    /// Stops playing, keeping what is shown.
    fn halt(&self) {
        if let Some(t) = self.timer.take()
            && let Some(ui) = self.ctl.ui.upgrade()
        {
            ui.cancel_timer(t);
        }
        self.wait.set(Wait::No);
        if self.running.replace(false) {
            self.stream().deactivate();
        }
    }

    /// Shows the frame the playhead is on, and arranges the next.
    fn present(self: &Rc<Self>, stream: &Arc<Stream>, size: (u32, u32)) {
        match stream.current() {
            Some(s) => {
                self.show(&s, size);
                self.schedule(stream);
            }
            None => {
                self.wait.set(Wait::Frame);
                if self.cur.borrow().is_none() && !self.poster_shown.get() {
                    self.show_poster(stream, size);
                }
            }
        }
    }

    fn go_on(self: &Rc<Self>, stream: &Arc<Stream>, size: (u32, u32)) {
        match self.wait.get() {
            Wait::No => {}
            Wait::Frame => self.present(stream, size),
            Wait::More => self.schedule(stream),
            Wait::Advance => self.advance(stream, size),
        }
    }

    fn show(&self, s: &Shown, size: (u32, u32)) {
        *self.cur.borrow_mut() = Some(s.clone());
        self.poster_shown.set(false);
        let _ = self.state.try_set(ImageState::Ready(s.image.clone(), size));
    }

    fn show_poster(&self, stream: &Arc<Stream>, size: (u32, u32)) {
        if let Some(p) = stream.poster() {
            *self.cur.borrow_mut() = None;
            self.poster_shown.set(true);
            let _ = self.state.try_set(ImageState::Ready(p, size));
        }
    }

    /// Sets the timer for the end of the frame shown, if another follows.
    fn schedule(self: &Rc<Self>, stream: &Arc<Stream>) {
        if let Some(t) = self.timer.take()
            && let Some(ui) = self.ctl.ui.upgrade()
        {
            ui.cancel_timer(t);
        }
        let Some(cur) = self.cur.borrow().clone() else { return };
        match stream.after(cur.seq) {
            Next::End => self.wait.set(Wait::No),
            Next::Wait => self.wait.set(Wait::More),
            Next::More => {
                self.wait.set(Wait::No);
                let held = Instant::now().saturating_duration_since(cur.started);
                let left = cur.delay.saturating_sub(held);
                // Timers count whole milliseconds: round up, never cut a frame short.
                let ms = u64::try_from(left.as_micros().div_ceil(1000)).unwrap_or(u64::MAX);
                let me = self.me.clone();
                let Some(ui) = self.ctl.ui.upgrade() else { return };
                let id = ui.after(Duration::from_millis(ms), move || {
                    if let Some(v) = me.upgrade() {
                        v.timer.set(None);
                        v.tick();
                    }
                });
                self.timer.set(Some(id));
            }
        }
    }

    /// The frame's time is up.
    fn tick(self: &Rc<Self>) {
        if self.dead.get() || !self.running.get() {
            return;
        }
        let stream = self.stream();
        if let Some(size) = stream.source_size() {
            self.advance(&stream, size);
        }
    }

    fn advance(self: &Rc<Self>, stream: &Arc<Stream>, size: (u32, u32)) {
        let Some(cur) = self.cur.borrow().clone() else {
            return self.present(stream, size);
        };
        match stream.step(cur.seq, Instant::now()) {
            Step::Show(s) => {
                self.show(&s, size);
                self.schedule(stream);
            }
            Step::Wait => self.wait.set(Wait::Advance),
            Step::End => self.wait.set(Wait::No),
        }
    }

    /// The element lets go of this view (it was removed, or grew and has a sharper one).
    fn shutdown(&self) {
        self.dead.set(true);
        self.halt();
        self.stream().unlisten(self.token.get());
    }
}
