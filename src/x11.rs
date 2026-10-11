//! Just enough of the X11 protocol to read and set a number on the root
//! window, which is how Steam talks to Gamescope in Game Mode (for example
//! `GAMESCOPE_DYNAMIC_REFRESH`, the refresh rate for the focused app). Plain
//! `std`, so the static CLI build stays dependency free.

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const CARDINAL: u32 = 6;
const OP_INTERN_ATOM: u8 = 16;
const OP_CHANGE_PROPERTY: u8 = 18;
const OP_DELETE_PROPERTY: u8 = 19;
const OP_GET_PROPERTY: u8 = 20;
const OP_GET_INPUT_FOCUS: u8 = 43;
/// Xauthority's "family" for local (unix socket) connections and "any".
const FAMILY_LOCAL: u16 = 256;
const FAMILY_WILD: u16 = 65535;

pub struct Display {
    stream: UnixStream,
    root: u32,
}

/// The display number in `$DISPLAY`: ":1", ":1.0", "unix:1".
pub fn display_number(display: &str) -> Option<u32> {
    let (host, rest) = display.rsplit_once(':')?;
    if !(host.is_empty() || host == "unix") {
        return None;
    }
    rest.split('.').next()?.parse().ok()
}

fn pad(n: usize) -> usize {
    (4 - n % 4) % 4
}

fn push_padded(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
    buf.extend(std::iter::repeat_n(0, pad(bytes.len())));
}

/// MIT-MAGIC-COOKIE-1 for `display` from an Xauthority file, if any.
pub fn find_cookie(xauth: &[u8], display: u32, hostname: &str) -> Option<Vec<u8>> {
    let mut at = 0;
    let field = |at: &mut usize| -> Option<Vec<u8>> {
        let len = u16::from_be_bytes([*xauth.get(*at)?, *xauth.get(*at + 1)?]) as usize;
        let value = xauth.get(*at + 2..*at + 2 + len)?.to_vec();
        *at += 2 + len;
        Some(value)
    };
    let number = display.to_string();
    let mut fallback = None;
    while at + 2 <= xauth.len() {
        let family = u16::from_be_bytes([xauth[at], xauth[at + 1]]);
        at += 2;
        let address = field(&mut at)?;
        let num = field(&mut at)?;
        let name = field(&mut at)?;
        let data = field(&mut at)?;
        if name != b"MIT-MAGIC-COOKIE-1" || !(num.is_empty() || num == number.as_bytes()) {
            continue;
        }
        match family {
            FAMILY_LOCAL if address == hostname.as_bytes() => return Some(data),
            FAMILY_LOCAL | FAMILY_WILD => fallback = fallback.or(Some(data)),
            _ => {}
        }
    }
    fallback
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn xauthority() -> Option<PathBuf> {
    std::env::var_os("XAUTHORITY")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".Xauthority")))
}

/// The connection setup request.
fn setup_request(cookie: Option<&[u8]>) -> Vec<u8> {
    let (name, data): (&[u8], &[u8]) = match cookie {
        Some(c) => (b"MIT-MAGIC-COOKIE-1", c),
        None => (b"", b""),
    };
    let mut buf = vec![b'l', 0];
    buf.extend_from_slice(&11u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
    buf.extend_from_slice(&(data.len() as u16).to_le_bytes());
    buf.extend_from_slice(&[0, 0]);
    push_padded(&mut buf, name);
    push_padded(&mut buf, data);
    buf
}

/// The root window of the first screen in a successful setup reply (the
/// bytes after the 8-byte header).
fn root_from_setup(body: &[u8]) -> Option<u32> {
    let vendor_len = u16::from_le_bytes([*body.get(16)?, *body.get(17)?]) as usize;
    let formats = *body.get(21)? as usize;
    let screens = 32 + vendor_len + pad(vendor_len) + formats * 8;
    let root = body.get(screens..screens + 4)?;
    Some(u32::from_le_bytes(root.try_into().ok()?))
}

fn intern_atom_request(name: &str) -> Vec<u8> {
    let len = 2 + (name.len() + pad(name.len())) / 4;
    let mut buf = vec![OP_INTERN_ATOM, 0];
    buf.extend_from_slice(&(len as u16).to_le_bytes());
    buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
    buf.extend_from_slice(&[0, 0]);
    push_padded(&mut buf, name.as_bytes());
    buf
}

fn change_property_request(window: u32, atom: u32, value: u32) -> Vec<u8> {
    let mut buf = vec![OP_CHANGE_PROPERTY, 0];
    buf.extend_from_slice(&7u16.to_le_bytes());
    for word in [window, atom, CARDINAL] {
        buf.extend_from_slice(&word.to_le_bytes());
    }
    buf.extend_from_slice(&[32, 0, 0, 0]);
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&value.to_le_bytes());
    buf
}

impl Display {
    /// Connect to `$DISPLAY` over its unix socket.
    pub fn open() -> Result<Display> {
        let display = std::env::var("DISPLAY").context("DISPLAY is not set")?;
        let number = display_number(&display)
            .with_context(|| format!("can't use DISPLAY={display} (only local displays)"))?;
        let cookie = xauthority()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|x| find_cookie(&x, number, &hostname()));
        match Display::connect(number, cookie.as_deref()) {
            Ok(d) => Ok(d),
            // Gamescope's Xwayland may not check credentials at all.
            Err(_) if cookie.is_some() => Display::connect(number, None),
            Err(e) => Err(e),
        }
    }

    fn connect(number: u32, cookie: Option<&[u8]>) -> Result<Display> {
        let path = format!("/tmp/.X11-unix/X{number}");
        let mut stream =
            UnixStream::connect(&path).with_context(|| format!("connecting to {path}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.write_all(&setup_request(cookie))?;
        let mut head = [0u8; 8];
        stream.read_exact(&mut head)?;
        let len = u16::from_le_bytes([head[6], head[7]]) as usize * 4;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body)?;
        if head[0] != 1 {
            let reason_len = head[1] as usize;
            let reason = String::from_utf8_lossy(body.get(..reason_len).unwrap_or_default());
            bail!("the X server refused the connection: {}", reason.trim());
        }
        let root = root_from_setup(&body).context("unreadable X setup reply")?;
        Ok(Display { stream, root })
    }

    /// Read one 32-byte reply, failing on an X error.
    fn reply(&mut self) -> Result<[u8; 32]> {
        loop {
            let mut buf = [0u8; 32];
            self.stream.read_exact(&mut buf)?;
            match buf[0] {
                0 => bail!("X error {} (request {})", buf[1], buf[10]),
                1 => return Ok(buf),
                // An event; nothing here asks for any, but skip it if one comes.
                _ => continue,
            }
        }
    }

    fn atom(&mut self, name: &str) -> Result<u32> {
        self.stream.write_all(&intern_atom_request(name))?;
        let r = self.reply()?;
        Ok(u32::from_le_bytes([r[8], r[9], r[10], r[11]]))
    }

    /// Wait until the server has handled everything sent so far.
    fn sync(&mut self) -> Result<()> {
        self.stream.write_all(&[OP_GET_INPUT_FOCUS, 0, 1, 0])?;
        self.reply().map(drop)
    }

    /// Set a 32-bit CARDINAL property on the root window.
    pub fn set_root(&mut self, name: &str, value: u32) -> Result<()> {
        let atom = self.atom(name)?;
        self.stream
            .write_all(&change_property_request(self.root, atom, value))?;
        self.sync()
    }

    pub fn delete_root(&mut self, name: &str) -> Result<()> {
        let atom = self.atom(name)?;
        let mut buf = vec![OP_DELETE_PROPERTY, 0];
        buf.extend_from_slice(&3u16.to_le_bytes());
        buf.extend_from_slice(&self.root.to_le_bytes());
        buf.extend_from_slice(&atom.to_le_bytes());
        self.stream.write_all(&buf)?;
        self.sync()
    }

    /// A 32-bit CARDINAL property of the root window, if it is set.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub fn get_root(&mut self, name: &str) -> Result<Option<u32>> {
        let atom = self.atom(name)?;
        let mut buf = vec![OP_GET_PROPERTY, 0];
        buf.extend_from_slice(&6u16.to_le_bytes());
        for word in [self.root, atom, CARDINAL, 0, 1] {
            buf.extend_from_slice(&word.to_le_bytes());
        }
        self.stream.write_all(&buf)?;
        let r = self.reply()?;
        let extra = u32::from_le_bytes([r[4], r[5], r[6], r[7]]) as usize * 4;
        let mut data = vec![0u8; extra];
        self.stream.read_exact(&mut data)?;
        let (format, items) = (r[1], u32::from_le_bytes([r[16], r[17], r[18], r[19]]));
        if format != 32 || items == 0 || data.len() < 4 {
            return Ok(None);
        }
        Ok(Some(u32::from_le_bytes([
            data[0], data[1], data[2], data[3],
        ])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_numbers() {
        assert_eq!(display_number(":0"), Some(0));
        assert_eq!(display_number(":1.0"), Some(1));
        assert_eq!(display_number("unix:2"), Some(2));
        assert_eq!(display_number("host:0"), None);
        assert_eq!(display_number(""), None);
    }

    #[test]
    fn requests_are_word_aligned() {
        assert_eq!(setup_request(None).len(), 12);
        assert_eq!(setup_request(Some(&[7; 16])).len(), 12 + 20 + 16);
        let atom = intern_atom_request("GAMESCOPE_DYNAMIC_REFRESH");
        assert_eq!(atom.len() % 4, 0);
        assert_eq!(
            u16::from_le_bytes([atom[2], atom[3]]) as usize * 4,
            atom.len()
        );
        let change = change_property_request(0x123, 0x456, 120);
        assert_eq!(change.len(), 28);
        assert_eq!(&change[24..], &120u32.to_le_bytes());
    }

    #[test]
    fn xauthority_cookies() {
        fn entry(family: u16, addr: &str, num: &str, name: &str, data: &[u8]) -> Vec<u8> {
            let mut out = family.to_be_bytes().to_vec();
            for f in [addr.as_bytes(), num.as_bytes(), name.as_bytes(), data] {
                out.extend_from_slice(&(f.len() as u16).to_be_bytes());
                out.extend_from_slice(f);
            }
            out
        }
        let mut file = entry(FAMILY_LOCAL, "other", "0", "MIT-MAGIC-COOKIE-1", &[1; 16]);
        file.extend(entry(
            FAMILY_LOCAL,
            "steamdeck",
            "1",
            "MIT-MAGIC-COOKIE-1",
            &[2; 16],
        ));
        file.extend(entry(
            FAMILY_LOCAL,
            "steamdeck",
            "0",
            "MIT-MAGIC-COOKIE-1",
            &[3; 16],
        ));
        assert_eq!(find_cookie(&file, 0, "steamdeck"), Some(vec![3; 16]));
        assert_eq!(find_cookie(&file, 1, "steamdeck"), Some(vec![2; 16]));
        assert_eq!(find_cookie(&file, 0, "elsewhere"), Some(vec![1; 16]));
        assert_eq!(find_cookie(&file, 5, "steamdeck"), None);
        assert_eq!(find_cookie(&[0, 1, 0], 0, "x"), None);
    }

    #[test]
    fn setup_reply_root() {
        // 32 fixed bytes, a 5-byte vendor (padded to 8), one 8-byte format,
        // then the first screen starting with its root window.
        let mut body = vec![0u8; 32];
        body[16] = 5;
        body[21] = 1;
        body.extend_from_slice(b"Vendr\0\0\0");
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&0x0000_0539u32.to_le_bytes());
        assert_eq!(root_from_setup(&body), Some(0x539));
        assert_eq!(root_from_setup(&body[..40]), None);
    }
}
