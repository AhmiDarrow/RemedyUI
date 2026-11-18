//! A carousel: pages that slide, with arrows, dots, swipe and the keyboard.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_a11y::{Action, Live, Role, Semantics};
use remedy_ui_core::{CornerRadii, Insets, Point, Rect};
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_layout::{Align, Display, Justify, Length, Position, Track};
use remedy_ui_paint::Brush;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, RichText, Ui};
use remedy_ui_viz::{Clock, Moved, Ticker};
use remedy_ui_widgets_core as w;

/// How a [`carousel`] behaves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CarouselOptions {
    /// Whether the last page leads on to the first and the first back to the last.
    pub wrap: bool,
    /// Previous and next buttons.
    pub arrows: bool,
    /// A dot for each page, to jump to it.
    pub dots: bool,
    /// Move on by itself after this long on a page. It stops while the pointer is over the
    /// carousel or focus is inside it, can be paused with a button, and never runs under
    /// reduced motion.
    pub autoplay: Option<Duration>,
    /// How long a slide takes. `None` takes the theme's standard motion time. Under reduced
    /// motion pages change at once.
    pub transition: Option<Duration>,
    /// Where time comes from.
    pub clock: Clock,
}

impl CarouselOptions {
    /// Sets `wrap`: whether the last page leads on to the first and the first back to the last.
    pub fn wrap(mut self, value: bool) -> Self {
        self.wrap = value;
        self
    }

    /// Sets `arrows`: previous and next buttons.
    pub fn arrows(mut self, value: bool) -> Self {
        self.arrows = value;
        self
    }

    /// Sets `dots`: a dot for each page, to jump to it.
    pub fn dots(mut self, value: bool) -> Self {
        self.dots = value;
        self
    }

    /// Sets `autoplay`: move on by itself after this long on a page.
    pub fn autoplay(mut self, value: Duration) -> Self {
        self.autoplay = Some(value);
        self
    }

    /// Sets `transition`: how long a slide takes.
    pub fn transition(mut self, value: Duration) -> Self {
        self.transition = Some(value);
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

impl Default for CarouselOptions {
    fn default() -> Self {
        CarouselOptions {
            wrap: false,
            arrows: true,
            dots: true,
            autoplay: None,
            transition: None,
            clock: Clock::default(),
        }
    }
}

/// The autoplay step, kept where the timer it sets can find it again.
type Schedule = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// Where the pages are while they slide.
struct Motion {
    /// The page shown, as a fraction (2.5 is halfway between pages 2 and 3).
    pos: f32,
    /// The page it is heading to.
    target: usize,
    /// A finger or mouse is dragging the pages.
    dragging: bool,
    /// The width of the viewport.
    width: f32,
}

/// Where a drag began and where it last was.
#[derive(Clone, Copy)]
struct Drag {
    x0: f32,
    pos0: f32,
    last_x: f32,
    last_t: Duration,
}

/// Fast enough to count as a flick, in logical pixels per second.
const FLICK: f32 = 600.0;

/// The signed distance from `pos` to page `i`, going the short way round when pages wrap.
fn delta(i: usize, pos: f32, n: usize, wrap: bool) -> f32 {
    let d = i as f32 - pos;
    if !wrap || n < 2 {
        return d;
    }
    let n = n as f32;
    let d = d.rem_euclid(n);
    if d > n / 2.0 { d - n } else { d }
}

fn visible(d: f32) -> bool {
    d.abs() < 1.0 - 1e-4
}

/// The page a drag that ended at `pos` settles on.
fn settle(pos: f32, flick: f32, n: usize, wrap: bool) -> usize {
    let to = if flick < -FLICK {
        pos.floor() + 1.0
    } else if flick > FLICK {
        pos.ceil() - 1.0
    } else {
        pos.round()
    };
    let to = if wrap { to.rem_euclid(n.max(1) as f32) } else { to.clamp(0.0, n.saturating_sub(1) as f32) };
    to as usize
}

/// Pages in a viewport, one at a time. `index` is the page shown; changing it from anywhere
/// slides there. Pages move with the pointer when dragged (let go past a quarter of the
/// width, or with a flick, to turn the page), with the Left and Right arrow keys while the
/// carousel has focus (Up, Down, Home, End, Page Up and Page Down too), and with the arrow
/// and dot buttons.
///
/// The carousel is a group named for its label that reads "Page 2 of 5" and announces
/// changes politely. Pages that are not shown are hidden from assistive technology and the
/// keyboard. In right-to-left windows pages run from the right.
#[track_caller]
pub fn carousel(
    ui: &Ui,
    label: impl Into<String>,
    pages: Vec<El>,
    index: Signal<usize>,
    options: CarouselOptions,
) -> El {
    let label = label.into();
    let n = pages.len();
    let unit = ui.theme().spacing.unit;
    let wrap = options.wrap;
    let transition = options.transition.unwrap_or(ui.theme().motion.standard);
    let start_index = index.get_untracked().unwrap_or(0).min(n.saturating_sub(1));
    let motion = Rc::new(RefCell::new(Motion {
        pos: start_index as f32,
        target: start_index,
        dragging: false,
        width: 0.0,
    }));

    // The viewport: every page in one grid cell, shifted sideways by where it is.
    let viewport = ui
        .el("carousel_viewport")
        .clip()
        .keyboard_alternative("Arrow keys on the carousel, or the previous and next buttons")
        .layout(|l| {
            l.display = Display::Grid;
            l.columns = vec![Track::Fr(1.0)];
            l.width = Length::Percent(1.0);
        });
    let wrappers: Vec<El> = pages
        .into_iter()
        .enumerate()
        .map(|(i, page)| {
            ui.el("carousel_page")
                .layout(|l| {
                    l.grid_column = Some((1, 1));
                    l.grid_row = Some((1, 1));
                    l.position = Position::Relative;
                    // A page is as tall as its content, so its height can be read and slid between.
                    l.align_self = Some(Align::Start);
                })
                .child(page)
                .hidden(i != start_index)
                .semantics(Semantics::new(Role::Group).label(format!("Page {} of {n}", i + 1)))
        })
        .collect();
    let viewport = wrappers.iter().fold(viewport, |v, p| v.child(p.clone()));

    // Puts every page where the motion says, and hides the ones out of view. While two pages
    // are in view the viewport is as tall as the two blended by how far each has come, so a
    // slide between pages of different heights grows or shrinks smoothly; at rest it is as
    // tall as its page (and follows that page if it changes).
    let place = {
        let wrappers = wrappers.clone();
        let motion = motion.clone();
        let viewport = viewport.clone();
        let last_height = Rc::new(Cell::new(0.0f32));
        move || {
            let Ok(m) = motion.try_borrow() else { return };
            // (weight, natural height when laid out) of each page in view.
            let mut in_view: Vec<(f32, Option<f32>)> = Vec::new();
            for (i, p) in wrappers.iter().enumerate() {
                let d = delta(i, m.pos, n, wrap);
                let show = visible(d);
                let _ = p.clone().hidden(!show);
                if show {
                    let off = d * m.width;
                    let _ = p.clone().layout(|l| {
                        l.inset = Insets { start: off, end: -off, top: 0.0, bottom: 0.0 };
                    });
                    in_view.push((1.0 - d.abs(), p.bounds().map(|b| b.height()).filter(|h| *h > 0.0)));
                }
            }
            if in_view.len() <= 1 {
                if let Some((_, Some(h))) = in_view.first() {
                    last_height.set(*h);
                }
                let _ = viewport.clone().layout(|l| l.height = Length::Auto);
            } else {
                let total: f32 = in_view.iter().map(|(w, _)| w).sum();
                let known: Option<f32> = in_view
                    .iter()
                    .map(|(w, h)| h.map(|h| w * h))
                    .sum::<Option<f32>>()
                    .map(|sum| sum / total.max(f32::EPSILON));
                // Until the page coming in has been laid out, hold the height it left.
                if let Some(h) = known {
                    last_height.set(h);
                }
                let h = last_height.get();
                if h > 0.0 {
                    let _ = viewport.clone().layout(|l| l.height = Length::Px(h));
                }
            }
        }
    };

    let ticker = {
        let motion = motion.clone();
        let place = place.clone();
        let el = viewport.clone();
        Ticker::new(&viewport, options.clock.clone(), move |dt| {
            let reduced = el.ui().reduced_motion();
            let Ok(mut m) = motion.try_borrow_mut() else { return Moved::default() };
            if m.dragging {
                return Moved::default();
            }
            let goal = m.target as f32;
            let remaining = if wrap && n > 1 { delta(m.target, m.pos, n, true) } else { goal - m.pos };
            if reduced || transition.is_zero() || remaining.abs() < 1e-4 {
                m.pos = goal;
                drop(m);
                place();
                return Moved::default();
            }
            // A page takes `transition`; the slide eases out toward the end.
            let speed = dt.as_secs_f32() / transition.as_secs_f32();
            let advance = (speed * remaining.abs().clamp(0.2, 1.0) * 2.0).min(remaining.abs());
            let next = m.pos + remaining.signum() * advance;
            m.pos = if wrap { next.rem_euclid(n as f32) } else { next };
            let done = delta(m.target, m.pos, n, wrap).abs() < 1e-3;
            if done {
                m.pos = goal;
            }
            drop(m);
            place();
            Moved { changed: false, pending: !done }
        })
    };

    // Autoplay: one timer at a time, only while nothing holds it.
    let playing = Signal::new(true);
    let holds = Rc::new(Cell::new(0i32));
    let timer: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let schedule: Schedule = Rc::new(RefCell::new(None));
    {
        let (timer, holds, schedule2, el, every) =
            (timer.clone(), holds.clone(), schedule.clone(), viewport.clone(), options.autoplay);
        let f: Rc<dyn Fn()> = Rc::new(move || {
            let Some(every) = every else { return };
            let ui = el.ui().clone();
            if timer.get().is_some()
                || holds.get() > 0
                || ui.reduced_motion()
                || !playing.get_untracked().unwrap_or(false)
                || n < 2
                || !ui.tree().contains(el.id())
            {
                return;
            }
            let (timer2, holds2, schedule3, el2) =
                (timer.clone(), holds.clone(), schedule2.clone(), el.clone());
            let id = ui.after(every, move || {
                timer2.set(None);
                let ui = el2.ui().clone();
                if !ui.tree().contains(el2.id())
                    || holds2.get() > 0
                    || ui.reduced_motion()
                    || !playing.get_untracked().unwrap_or(false)
                {
                    return;
                }
                let cur = index.get_untracked().unwrap_or(0);
                let next = if cur + 1 < n {
                    cur + 1
                } else if wrap {
                    0
                } else {
                    return;
                };
                index.set(next);
                let again = schedule3.borrow().clone();
                if let Some(f) = again {
                    f();
                }
            });
            timer.set(Some(id));
        });
        *schedule.borrow_mut() = Some(f);
    }
    let run_schedule = {
        let schedule = schedule.clone();
        move || {
            let f = schedule.borrow().clone();
            if let Some(f) = f {
                f();
            }
        }
    };
    let cancel = {
        let timer = timer.clone();
        let el = viewport.clone();
        move || {
            if let Some(id) = timer.take() {
                el.ui().cancel_timer(id);
            }
        }
    };
    // The pointer over, or focus inside, holds autoplay.
    let hold = {
        let (holds, cancel, run) = (holds.clone(), cancel.clone(), run_schedule.clone());
        move |on: bool| {
            holds.set((holds.get() + if on { 1 } else { -1 }).max(0));
            if on {
                cancel();
            } else {
                run();
            }
        }
    };
    let with_hold = |el: El| -> El {
        let hold = hold.clone();
        el.on_focus_change(hold)
    };

    let go = {
        let cancel = cancel.clone();
        move |to: usize| {
            cancel();
            index.set_if_changed(to.min(n.saturating_sub(1)));
        }
    };
    let step_by = {
        let go = go.clone();
        move |by: i64| {
            let nn = n as i64;
            if nn == 0 {
                return;
            }
            let next = index.get_untracked().unwrap_or(0) as i64 + by;
            let next = if wrap { next.rem_euclid(nn) } else { next.clamp(0, nn - 1) };
            go(next as usize);
        }
    };

    let prev_btn = with_hold(w::icon_button(ui, w::Icon::ChevronLeft, "Previous page", {
        let step_by = step_by.clone();
        move || step_by(-1)
    }));
    let next_btn = with_hold(w::icon_button(ui, w::Icon::ChevronRight, "Next page", {
        let step_by = step_by.clone();
        move || step_by(1)
    }));
    let dots: Vec<El> = (0..n)
        .map(|i| {
            let go = go.clone();
            let dot = ui.el("carousel_dot").width(unit * 3.0).height(unit * 3.0).paint(move |cx| {
                let on = index.get() == i;
                let color = if on { cx.theme.colors.accent } else { cx.theme.derive().line };
                let r = cx.size.width.min(cx.size.height) / 2.0 * if on { 1.0 } else { 0.7 };
                let mid = Point::new(cx.size.width / 2.0, cx.size.height / 2.0);
                cx.list.fill_rounded(
                    Rect::new(mid.x - r, mid.y - r, mid.x + r, mid.y + r),
                    CornerRadii::all(r),
                    Brush::Solid(color),
                );
            });
            // The dot is small; the pressable part around it is a full target.
            let target = ui
                .el("carousel_dot_target")
                .row()
                .align(Align::Center)
                .justify(Justify::Center)
                .layout(|l| {
                    l.min_width = Length::Px(w::MIN_TARGET);
                    l.min_height = Length::Px(w::MIN_TARGET);
                })
                .child(dot)
                .on_click(move || go(i))
                .semantics_dyn(move || {
                    Semantics::new(Role::Button)
                        .label(format!("Go to page {} of {n}", i + 1))
                        .selected(index.get() == i)
                        .action(Action::Click)
                });
            with_hold(target)
        })
        .collect();
    let pause_btn = options.autoplay.map(|_| {
        let btn = with_hold(w::button(ui, "Pause", move || playing.update(|p| *p = !*p)));
        let (b2, cancel, run) = (btn.clone(), cancel.clone(), run_schedule.clone());
        let first = Cell::new(true);
        btn.bind(move || {
            let on = playing.get();
            let words = if on { "Pause" } else { "Play" };
            let _ = b2.clone().semantics(
                Semantics::new(Role::Button)
                    .label(format!("{words} automatic page changes"))
                    .action(Action::Click),
            );
            for c in b2.child_elements() {
                let _ = c.text(RichText::plain(words));
            }
            // The carousel itself starts the first timer.
            if first.replace(false) {
                return;
            }
            if on {
                run();
            } else {
                cancel();
            }
        })
    });

    let mut footer = w::row(ui).gap(unit).align(Align::Center).justify(Justify::Center);
    if options.arrows {
        footer = footer.child(prev_btn.clone());
    }
    if options.dots {
        footer = footer.children(dots);
    }
    if options.arrows {
        footer = footer.child(next_btn.clone());
    }
    if let Some(p) = pause_btn {
        footer = footer.child(p);
    }

    // The pointer drags the pages along, then they settle on the nearest.
    let drag: Rc<Cell<Option<Drag>>> = Rc::new(Cell::new(None));
    let viewport = {
        let (motion, place, ticker, cancel, el, drag) =
            (motion.clone(), place.clone(), ticker.clone(), cancel.clone(), viewport.clone(), drag.clone());
        viewport.on_pointer(move |e| match e.phase {
            PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                let Ok(mut m) = motion.try_borrow_mut() else { return false };
                if let Some(b) = el.bounds() {
                    m.width = b.width();
                }
                m.dragging = true;
                drag.set(Some(Drag { x0: e.position.x, pos0: m.pos, last_x: e.position.x, last_t: e.time }));
                cancel();
                false
            }
            PointerPhase::Move => {
                let Some(d) = drag.get() else { return false };
                let Ok(mut m) = motion.try_borrow_mut() else { return false };
                let dx = if el.is_rtl() { d.x0 - e.position.x } else { e.position.x - d.x0 };
                let mut pos = d.pos0 - dx / m.width.max(1.0);
                if wrap {
                    pos = pos.rem_euclid(n.max(1) as f32);
                } else {
                    // Pull against the ends rather than past them.
                    let top = n.saturating_sub(1) as f32;
                    if pos < 0.0 {
                        pos *= 0.3;
                    } else if pos > top {
                        pos = top + (pos - top) * 0.3;
                    }
                }
                m.pos = pos;
                drag.set(Some(Drag { last_x: e.position.x, last_t: e.time, ..d }));
                drop(m);
                place();
                true
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                let Some(d) = drag.take() else { return false };
                let Ok(mut m) = motion.try_borrow_mut() else { return false };
                m.dragging = false;
                let dt = e.time.saturating_sub(d.last_t).as_secs_f32();
                let v = if dt > 0.0 && dt < 0.15 { (e.position.x - d.last_x) / dt } else { 0.0 };
                let flick = if el.is_rtl() { -v } else { v };
                let moved = (e.position.x - d.x0).abs() > 2.0;
                let target = settle(m.pos, flick, n, wrap);
                m.target = target;
                drop(m);
                index.set_if_changed(target);
                ticker.kick();
                moved
            }
            _ => false,
        })
    };

    let size_motion = motion.clone();
    let size_place = place.clone();
    let viewport = viewport.on_resize(move |size| {
        if let Ok(mut m) = size_motion.try_borrow_mut() {
            m.width = size.width;
        }
        size_place();
    });

    let root = w::column(ui).gap(unit * 2.0).child(viewport.clone()).child(footer).focusable(true);
    let root = with_hold(root);
    let (bind_root, bind_prev, bind_next) = (root.clone(), prev_btn, next_btn);
    let root = root.bind(move || {
        let idx = index.get().min(n.saturating_sub(1));
        if let Ok(mut m) = motion.try_borrow_mut() {
            if m.width <= 0.0
                && let Some(b) = viewport.bounds()
            {
                m.width = b.width();
            }
            m.target = idx;
        }
        if !wrap {
            let _ = bind_prev.clone().disabled(idx == 0);
            let _ = bind_next.clone().disabled(idx + 1 >= n);
        }
        place();
        ticker.kick();
        cancel();
        run_schedule();
        let _ = bind_root.clone().semantics(
            Semantics::new(Role::Group)
                .label(label.clone())
                .value(format!("Page {} of {n}", idx + 1))
                .live(Live::Polite)
                .action(Action::Increment)
                .action(Action::Decrement),
        );
    });

    let key_el = root.clone();
    root.on_key(move |e| {
        if !e.pressed {
            return false;
        }
        let (back, forward) = if key_el.is_rtl() {
            (Key::ArrowRight, Key::ArrowLeft)
        } else {
            (Key::ArrowLeft, Key::ArrowRight)
        };
        let k = &e.key;
        if *k == back || *k == Key::ArrowUp || *k == Key::PageUp {
            step_by(-1);
        } else if *k == forward || *k == Key::ArrowDown || *k == Key::PageDown {
            step_by(1);
        } else if *k == Key::Home {
            go(0);
        } else if *k == Key::End {
            go(n.saturating_sub(1));
        } else {
            return false;
        }
        true
    })
    .on_hover(hold)
}
