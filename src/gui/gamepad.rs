//! Controller input for Game Mode.
//!
//! egui has no gamepad support and Steam's default layout for a non-Steam
//! game sends gamepad events, not keys, so the app reads the controller
//! itself from `/dev/input` (the same nodes games use) and turns it into the
//! keys the UI already understands:
//!
//! | Controller          | Key                     | Does                       |
//! |---------------------|-------------------------|----------------------------|
//! | D-pad, left stick   | arrows (repeat on hold) | move focus, step sliders   |
//! | A                   | Enter                   | press the focused control  |
//! | B                   | Escape                  | close a dialog, drop focus |
//! | LB / RB             | PageUp / PageDown       | previous / next page       |
//!
//! The first input with nothing focused focuses the first control instead.
//! Devices Steam asks games to ignore (`SDL_GAMECONTROLLER_IGNORE_DEVICES`,
//! the physical pad behind Steam's virtual one) are skipped so nothing
//! arrives twice.

use eframe::egui::{self, Event, Key, Modifiers, RawInput};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

/// `struct input_event` on 64-bit Linux: a 16-byte timeval, then type, code
/// and value.
const EVENT_SIZE: usize = 24;

/// Xbox-style sticks report -32768..32767. Past this a direction counts as
/// held; it has to come back under `STICK_RELEASE` to let go.
const STICK_PRESS: i32 = 20_000;
const STICK_RELEASE: i32 = 12_000;

const REPEAT_DELAY: Duration = Duration::from_millis(380);
const REPEAT_EVERY: Duration = Duration::from_millis(120);
const RESCAN_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    fn key(self) -> Key {
        match self {
            Dir::Up => Key::ArrowUp,
            Dir::Down => Key::ArrowDown,
            Dir::Left => Key::ArrowLeft,
            Dir::Right => Key::ArrowRight,
        }
    }
}

/// What the reader threads saw; the UI thread turns it into keys.
#[derive(Default)]
struct Shared {
    presses: Vec<Key>,
    dpad: [bool; 4],
    hat: (i32, i32),
    stick: (i32, i32),
    stick_dir: Option<Dir>,
    devices: usize,
}

impl Shared {
    /// D-pad buttons and hat first, then the stick.
    fn held(&self) -> Option<Dir> {
        let [up, down, left, right] = self.dpad;
        let (hx, hy) = self.hat;
        if up || hy < 0 {
            Some(Dir::Up)
        } else if down || hy > 0 {
            Some(Dir::Down)
        } else if left || hx < 0 {
            Some(Dir::Left)
        } else if right || hx > 0 {
            Some(Dir::Right)
        } else {
            self.stick_dir
        }
    }

    fn apply(&mut self, kind: u16, code: u16, value: i32) {
        match (kind, code) {
            (EV_KEY, _) if value == 1 => match code {
                BTN_SOUTH => self.presses.push(Key::Enter),
                BTN_EAST => self.presses.push(Key::Escape),
                BTN_TL => self.presses.push(Key::PageUp),
                BTN_TR => self.presses.push(Key::PageDown),
                _ => self.set_dpad(code, true),
            },
            (EV_KEY, _) if value == 0 => self.set_dpad(code, false),
            (EV_ABS, ABS_HAT0X) => self.hat.0 = value.signum(),
            (EV_ABS, ABS_HAT0Y) => self.hat.1 = value.signum(),
            (EV_ABS, ABS_X) => {
                self.stick.0 = value;
                self.update_stick();
            }
            (EV_ABS, ABS_Y) => {
                self.stick.1 = value;
                self.update_stick();
            }
            _ => {}
        }
    }

    fn set_dpad(&mut self, code: u16, down: bool) {
        let slot = match code {
            BTN_DPAD_UP => 0,
            BTN_DPAD_DOWN => 1,
            BTN_DPAD_LEFT => 2,
            BTN_DPAD_RIGHT => 3,
            _ => return,
        };
        self.dpad[slot] = down;
    }

    fn update_stick(&mut self) {
        let (x, y) = self.stick;
        let (ax, ay) = (x.saturating_abs(), y.saturating_abs());
        if let Some(dir) = self.stick_dir {
            let still = match dir {
                Dir::Up => -y,
                Dir::Down => y,
                Dir::Left => -x,
                Dir::Right => x,
            };
            if still >= STICK_RELEASE {
                return;
            }
        }
        self.stick_dir = if ax.max(ay) < STICK_PRESS {
            None
        } else if ay >= ax {
            Some(if y < 0 { Dir::Up } else { Dir::Down })
        } else {
            Some(if x < 0 { Dir::Left } else { Dir::Right })
        };
    }
}

pub struct Gamepad {
    shared: Arc<Mutex<Shared>>,
    repeating: Option<(Dir, Instant)>,
}

impl Gamepad {
    /// Start watching `/dev/input` for controllers.
    pub fn start(ctx: &egui::Context) -> Gamepad {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let watcher = Arc::clone(&shared);
        let ctx = ctx.clone();
        let _ = thread::Builder::new()
            .name("gamepad-scan".into())
            .spawn(move || scan_loop(&watcher, &ctx));
        Gamepad {
            shared,
            repeating: None,
        }
    }

    /// True once a controller is being read.
    pub fn connected(&self) -> bool {
        self.shared.lock().is_ok_and(|s| s.devices > 0)
    }

    /// Add this frame's controller input to egui's input as key presses.
    pub fn feed(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
        let (presses, held) = {
            let Ok(mut shared) = self.shared.lock() else {
                return;
            };
            (std::mem::take(&mut shared.presses), shared.held())
        };
        let mut focused = ctx.memory(|m| m.focused().is_some());
        for key in presses {
            // Escape with nothing focused would do nothing; Enter would press
            // nothing. Focus the first control instead.
            if !focused && key == Key::Enter {
                tap(raw, Key::Tab);
                focused = true;
            } else {
                tap(raw, key);
            }
        }
        let now = Instant::now();
        match (held, self.repeating) {
            (None, _) => self.repeating = None,
            (Some(dir), Some((was, next))) if dir == was => {
                if now >= next {
                    tap(raw, if focused { dir.key() } else { Key::Tab });
                    self.repeating = Some((dir, now + REPEAT_EVERY));
                }
            }
            (Some(dir), _) => {
                tap(raw, if focused { dir.key() } else { Key::Tab });
                self.repeating = Some((dir, now + REPEAT_DELAY));
            }
        }
        if let Some((_, next)) = self.repeating {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }
}

fn tap(raw: &mut RawInput, key: Key) {
    for pressed in [true, false] {
        raw.events.push(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
    }
}

/// Look for new controllers every few seconds and read each on its own thread.
fn scan_loop(shared: &Arc<Mutex<Shared>>, ctx: &egui::Context) {
    let ignored =
        ignored_ids(&std::env::var("SDL_GAMECONTROLLER_IGNORE_DEVICES").unwrap_or_default());
    let reading: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    loop {
        for node in gamepad_nodes(Path::new("/sys/class/input"), &ignored) {
            let Ok(mut open) = reading.lock() else {
                return;
            };
            if open.contains(&node) {
                continue;
            }
            let Ok(file) = File::open(&node) else {
                continue;
            };
            open.insert(node.clone());
            drop(open);
            let (shared, reading, ctx) = (Arc::clone(shared), Arc::clone(&reading), ctx.clone());
            let _ = thread::Builder::new()
                .name("gamepad".into())
                .spawn(move || read_device(file, &node, &shared, &reading, &ctx));
        }
        thread::sleep(RESCAN_EVERY);
    }
}

fn read_device(
    mut file: File,
    node: &Path,
    shared: &Mutex<Shared>,
    reading: &Mutex<HashSet<PathBuf>>,
    ctx: &egui::Context,
) {
    if let Ok(mut s) = shared.lock() {
        s.devices += 1;
    }
    let mut buf = [0u8; EVENT_SIZE];
    while file.read_exact(&mut buf).is_ok() {
        let kind = u16::from_ne_bytes([buf[16], buf[17]]);
        let code = u16::from_ne_bytes([buf[18], buf[19]]);
        let value = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        if kind != EV_KEY && kind != EV_ABS {
            continue;
        }
        if let Ok(mut s) = shared.lock() {
            s.apply(kind, code, value);
        }
        ctx.request_repaint();
    }
    // Unplugged: forget its held state and let a rescan pick it up again.
    if let Ok(mut s) = shared.lock() {
        s.devices = s.devices.saturating_sub(1);
        s.dpad = [false; 4];
        s.hat = (0, 0);
        s.stick = (0, 0);
        s.stick_dir = None;
    }
    if let Ok(mut r) = reading.lock() {
        r.remove(node);
    }
}

/// `/dev/input/eventN` of every device with gamepad buttons, minus ignored ones.
fn gamepad_nodes(class: &Path, ignored: &HashSet<(u16, u16)>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(class) else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("event") {
                return None;
            }
            let device = e.path().join("device");
            let keys = fs::read_to_string(device.join("capabilities/key")).ok()?;
            if !has_bit(&keys, BTN_SOUTH) {
                return None;
            }
            let id = |f: &str| {
                fs::read_to_string(device.join("id").join(f))
                    .ok()
                    .and_then(|v| u16::from_str_radix(v.trim(), 16).ok())
            };
            if let (Some(vendor), Some(product)) = (id("vendor"), id("product"))
                && ignored.contains(&(vendor, product))
            {
                return None;
            }
            Some(Path::new("/dev/input").join(name))
        })
        .collect();
    nodes.sort();
    nodes
}

/// Whether a sysfs capability bitmap ("3f0000 0 0 ...", most significant
/// word first) has `bit` set.
fn has_bit(bitmap: &str, bit: u16) -> bool {
    let words: Vec<&str> = bitmap.split_whitespace().collect();
    let index = usize::from(bit) / 64;
    let Some(word) = words.len().checked_sub(index + 1).map(|i| words[i]) else {
        return false;
    };
    u64::from_str_radix(word, 16).is_ok_and(|w| (w >> (bit % 64)) & 1 == 1)
}

/// Steam's "0x28de/0x1205,0x0b05/0x1b4c" list of vendor/product ids.
fn ignored_ids(list: &str) -> HashSet<(u16, u16)> {
    let hex = |s: &str| {
        let s = s.trim();
        u16::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
    };
    list.split(',')
        .filter_map(|pair| {
            let (v, p) = pair.split_once('/')?;
            Some((hex(v)?, hex(p)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_bits() {
        // A typical gamepad: BTN_SOUTH (0x130) is bit 48 of the fifth word
        // from the end.
        let pad = "7cdb000000000000 0 0 0 0";
        assert!(has_bit(pad, BTN_SOUTH));
        assert!(!has_bit("120013", BTN_SOUTH));
        assert!(!has_bit("", BTN_SOUTH));
        assert!(has_bit("1", 0));
    }

    #[test]
    fn steam_ignore_list() {
        let ids = ignored_ids("0x28de/0x1205,0x0b05/0x1b4c, junk ,0x0B05/0x1ABE");
        assert!(ids.contains(&(0x28de, 0x1205)));
        assert!(ids.contains(&(0x0b05, 0x1b4c)));
        assert!(ids.contains(&(0x0b05, 0x1abe)));
        assert_eq!(ids.len(), 3);
        assert!(ignored_ids("").is_empty());
    }

    #[test]
    fn buttons_and_directions() {
        let mut s = Shared::default();
        s.apply(EV_KEY, BTN_SOUTH, 1);
        s.apply(EV_KEY, BTN_SOUTH, 0);
        s.apply(EV_KEY, BTN_EAST, 1);
        s.apply(EV_KEY, BTN_TR, 1);
        assert_eq!(s.presses, vec![Key::Enter, Key::Escape, Key::PageDown]);

        s.apply(EV_ABS, ABS_HAT0Y, 1);
        assert_eq!(s.held(), Some(Dir::Down));
        s.apply(EV_ABS, ABS_HAT0Y, 0);
        assert_eq!(s.held(), None);
        s.apply(EV_KEY, BTN_DPAD_LEFT, 1);
        assert_eq!(s.held(), Some(Dir::Left));
        s.apply(EV_KEY, BTN_DPAD_LEFT, 0);

        // The stick needs a firm push and lets go with some hysteresis.
        s.apply(EV_ABS, ABS_X, 15_000);
        assert_eq!(s.held(), None);
        s.apply(EV_ABS, ABS_X, 30_000);
        assert_eq!(s.held(), Some(Dir::Right));
        s.apply(EV_ABS, ABS_X, 14_000);
        assert_eq!(s.held(), Some(Dir::Right));
        s.apply(EV_ABS, ABS_X, 2_000);
        assert_eq!(s.held(), None);
        s.apply(EV_ABS, ABS_Y, -32_768);
        assert_eq!(s.held(), Some(Dir::Up));
    }
}
