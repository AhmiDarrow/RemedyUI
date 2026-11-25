//! Gamepads through evdev (/dev/input/event*), for the shared core's gamepad thread: pads
//! plugged in later are found through inotify. A device the user may not read (most
//! desktops grant the seated user their gamepads, not other people's devices) is passed over,
//! as is a machine without /dev/input (a sandbox without device access): no pads, no error.

use std::fs::File;
use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::bridge_winit::gamepad::PadSource;
use remedy_ui_core::input::PadState;

use crate::bridge_linux::evdev::{self, Event};
use crate::bridge_linux::os::gamepad as os;

const DIR: &str = "/dev/input";
/// How often /dev/input is looked at again when inotify is unavailable.
const RESCAN: Duration = Duration::from_secs(2);

struct Device {
    file: File,
    path: PathBuf,
    pad: evdev::Pad,
}

/// The pads under /dev/input, each in the slot it was given when found.
#[derive(Default)]
pub(crate) struct Evdev {
    watch: Option<File>,
    started: bool,
    /// When /dev/input was last looked at.
    scanned: Option<Instant>,
    pads: Vec<Option<Device>>,
}

/// Reads the device's state into `pad` (`EVIOCGKEY` and `EVIOCGABS`).
fn resync(file: &File, pad: &mut evdev::Pad) {
    let keys = os::key_bits(file, true).unwrap_or_default();
    pad.resync(&keys, evdev::AXES.map(|a| os::abs_info(file, a).map(|(v, _, _)| v)));
}

impl Evdev {
    /// Opens the gamepads not yet open, reporting each one's state.
    fn scan(&mut self, out: &mut Vec<(usize, Option<PadState>)>) {
        self.scanned = Some(Instant::now());
        let Ok(dir) = std::fs::read_dir(DIR) else { return };
        for entry in dir.flatten() {
            let path = entry.path();
            let is_event = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event"));
            if !is_event || self.pads.iter().flatten().any(|d| d.path == path) {
                continue;
            }
            if let Some(dev) = open(&path) {
                let state = dev.pad.state();
                let slot = match self.pads.iter().position(Option::is_none) {
                    Some(i) => i,
                    None => {
                        self.pads.push(None);
                        self.pads.len() - 1
                    }
                };
                if let Some(s) = self.pads.get_mut(slot) {
                    *s = Some(dev);
                    out.push((slot, Some(state)));
                }
            }
        }
    }

    /// Takes the events waiting on pad `slot`.
    fn drain(&mut self, slot: usize, out: &mut Vec<(usize, Option<PadState>)>) {
        let Some(Some(dev)) = self.pads.get_mut(slot) else { return };
        let mut buf = [0u8; os::EVENT_SIZE * 64];
        loop {
            match dev.file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut changed = false;
                    for e in buf.get(..n).unwrap_or_default().as_chunks::<{ os::EVENT_SIZE }>().0 {
                        // The type, code and value end every layout of input_event.
                        let tail = e.get(os::EVENT_SIZE - 8..).unwrap_or_default();
                        let u16_at = |i: usize| {
                            tail.get(i..i + 2).and_then(|b| b.try_into().ok()).map(u16::from_ne_bytes)
                        };
                        let value = tail.get(4..8).and_then(|b| b.try_into().ok()).map(i32::from_ne_bytes);
                        let (Some(ty), Some(code), Some(value)) = (u16_at(0), u16_at(2), value) else {
                            continue;
                        };
                        match dev.pad.event(ty, code, value) {
                            Event::None => {}
                            Event::Changed => changed = true,
                            Event::Resync => {
                                resync(&dev.file, &mut dev.pad);
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        out.push((slot, Some(dev.pad.state())));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                // ENODEV: it was unplugged.
                Err(_) => {
                    if let Some(s) = self.pads.get_mut(slot) {
                        *s = None;
                    }
                    out.push((slot, None));
                    break;
                }
            }
        }
    }
}

/// Opens `path` if it is a gamepad this user may read.
fn open(path: &Path) -> Option<Device> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    if !evdev::is_gamepad(&os::key_bits(&file, false)?) {
        return None;
    }
    let range = [evdev::ABS_X, evdev::ABS_Y].map(|a| os::abs_info(&file, a).map(|(_, min, max)| (min, max)));
    let mut pad = evdev::Pad::new(range);
    resync(&file, &mut pad);
    Some(Device { file, path: path.to_owned(), pad })
}

impl PadSource for Evdev {
    fn read(&mut self, wait: Option<Duration>) -> Vec<(usize, Option<PadState>)> {
        let mut out = Vec::new();
        if !std::mem::replace(&mut self.started, true) {
            self.watch = os::watch(c"/dev/input");
            self.scan(&mut out);
            if !out.is_empty() {
                return out;
            }
        }
        let wait = if self.watch.is_some() { wait } else { Some(wait.map_or(RESCAN, |w| w.min(RESCAN))) };
        let slots: Vec<usize> =
            (0..self.pads.len()).filter(|i| self.pads.get(*i).is_some_and(Option::is_some)).collect();
        let mut files: Vec<&File> = self.watch.iter().collect();
        files
            .extend(slots.iter().filter_map(|i| self.pads.get(*i).and_then(Option::as_ref)).map(|d| &d.file));
        let ready = os::poll(&files, wait);
        let mut ready = ready.into_iter();
        let rescan = match &mut self.watch {
            Some(w) => {
                let hit = ready.next().unwrap_or(false);
                if hit {
                    // What changed does not matter: /dev/input is looked at again.
                    let mut buf = [0u8; 4096];
                    while w.read(&mut buf).is_ok_and(|n| n > 0) {}
                }
                hit
            }
            None => self.scanned.is_none_or(|t| t.elapsed() >= RESCAN),
        };
        let hits: Vec<usize> = slots.into_iter().zip(ready).filter(|(_, r)| *r).map(|(s, _)| s).collect();
        for slot in hits {
            self.drain(slot, &mut out);
        }
        if rescan {
            self.scan(&mut out);
        }
        out
    }
}
