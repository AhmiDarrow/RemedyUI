//! What the input devices the kernel lists (`/proc/bus/input/devices`) are: mice and
//! touchpads, touch screens, pens, told apart by their capability bits as udev's input_id
//! does. Plain text in, so it is tested on every platform.

const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const ABS_X: u16 = 0x00;
const ABS_MT_POSITION_X: u16 = 0x35;
const BTN_LEFT: u16 = 0x110;
const BTN_TOOL_PEN: u16 = 0x140;
const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOUCH: u16 = 0x14a;
const INPUT_PROP_DIRECT: u16 = 0x01;

/// Whether bit `n` is set in a capability mask as the kernel prints it: hex words of
/// `word_bits` bits (the kernel's `long`), most significant first.
fn bit(mask: &str, n: u16, word_bits: u32) -> bool {
    let (word, at) = (u32::from(n) / word_bits, u32::from(n) % word_bits);
    mask.split_whitespace()
        .rev()
        .nth(word as usize)
        .and_then(|w| u64::from_str_radix(w, 16).ok())
        .is_some_and(|w| (w >> at) & 1 == 1)
}

/// The kinds of pointing device among those listed: (a mouse or touchpad, a touch screen, a
/// pen). `word_bits` is the kernel's `long` in bits.
pub(crate) fn kinds(devices: &str, word_bits: u32) -> (bool, bool, bool) {
    let (mut mouse, mut touch, mut pen) = (false, false, false);
    for block in devices.split("\n\n") {
        let mask = |name: &str| {
            let key = format!("B: {name}=");
            block.lines().find_map(|l| l.strip_prefix(key.as_str())).unwrap_or("")
        };
        let (key, rel, abs, prop) = (mask("KEY"), mask("REL"), mask("ABS"), mask("PROP"));
        let has = |m: &str, n| bit(m, n, word_bits);
        let positions = has(abs, ABS_X) || has(abs, ABS_MT_POSITION_X);
        let direct = has(prop, INPUT_PROP_DIRECT);
        if positions && has(key, BTN_TOOL_PEN) {
            pen = true;
        } else if positions && has(key, BTN_TOOL_FINGER) && !direct {
            // A touchpad: it moves a pointer that hovers.
            mouse = true;
        } else if positions && has(key, BTN_TOUCH) {
            touch = true;
        } else if ((has(rel, REL_X) && has(rel, REL_Y)) || positions) && has(key, BTN_LEFT) {
            // A mouse, a trackball, or an absolute pointer (a virtual machine's tablet).
            mouse = true;
        }
    }
    (mouse, touch, pen)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUSE: &str = "I: Bus=0003 Vendor=046d Product=c077 Version=0111
N: Name=\"Logitech USB Optical Mouse\"
H: Handlers=mouse0 event3
B: PROP=0
B: EV=17
B: KEY=ff0000 0 0 0 0
B: REL=903
B: MSC=10
";
    const KEYBOARD: &str = "I: Bus=0011 Vendor=0001 Product=0001 Version=ab83
N: Name=\"AT Translated Set 2 keyboard\"
H: Handlers=sysrq kbd event0 leds
B: PROP=0
B: EV=120013
B: KEY=402000000 3803078f800d001 feffffdfffefffff fffffffffffffffe
B: MSC=10
";
    const TOUCHSCREEN: &str = "I: Bus=0018 Vendor=04f3 Product=2a3c Version=0100
N: Name=\"ELAN Touchscreen\"
H: Handlers=mouse1 event5
B: PROP=2
B: EV=b
B: KEY=400 0 0 0 0 0
B: ABS=3273800000000003
";
    const PEN: &str = "I: Bus=0003 Vendor=056a Product=0374 Version=0110
N: Name=\"Wacom Intuos S Pen\"
H: Handlers=mouse2 event7
B: PROP=1
B: EV=1b
B: KEY=1c03 0 0 0 0 0
B: ABS=3000003
";
    const TOUCHPAD: &str = "I: Bus=0011 Vendor=0002 Product=0007 Version=01b1
N: Name=\"SynPS/2 Synaptics TouchPad\"
H: Handlers=mouse3 event4
B: PROP=5
B: EV=b
B: KEY=6420 30000 0 0 0 0
B: ABS=260800011000003
";

    fn list(devices: &[&str]) -> String {
        devices.join("\n")
    }

    #[test]
    fn devices_are_told_apart_by_their_bits() {
        assert_eq!(kinds(&list(&[KEYBOARD, MOUSE]), 64), (true, false, false));
        assert_eq!(kinds(&list(&[KEYBOARD, TOUCHSCREEN]), 64), (false, true, false), "a tablet");
        assert_eq!(kinds(&list(&[TOUCHPAD, TOUCHSCREEN, PEN]), 64), (true, true, true), "a convertible");
        assert_eq!(kinds(PEN, 64), (false, false, true));
        assert_eq!(kinds(KEYBOARD, 64), (false, false, false));
        assert_eq!(kinds("", 64), (false, false, false));
    }

    #[test]
    fn masks_follow_the_kernels_word_size() {
        let mouse32 = "B: KEY=ff0000 0 0 0 0 0 0 0 0\nB: REL=3\n";
        assert_eq!(kinds(mouse32, 32), (true, false, false));
        assert!(!bit("ff0000 0 0 0 0", 0x110, 32), "the same mask read as 32-bit words is something else");
        assert!(bit("ff0000 0 0 0 0", 0x110, 64));
    }
}
