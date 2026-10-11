//! Turning "2560x1440@170" into a concrete detailed timing.

use crate::edid::{Dtd, Edid};
use anyhow::{Context, Result, bail};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub hz: f64,
}

impl FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Mode> {
        let usage =
            || format!("invalid mode '{s}', expected WIDTHxHEIGHT@HZ such as 2560x1440@170");
        let (res, hz) = s.split_once('@').with_context(usage)?;
        let (w, h) = res.split_once(['x', 'X']).with_context(usage)?;
        let mode = Mode {
            width: w.trim().parse().with_context(usage)?,
            height: h.trim().parse().with_context(usage)?,
            hz: hz
                .trim()
                .trim_end_matches("Hz")
                .parse()
                .with_context(usage)?,
        };
        if mode.width == 0 || mode.height == 0 || !(1.0..=1000.0).contains(&mode.hz) {
            bail!(usage());
        }
        Ok(mode)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}@{}", self.width, self.height, self.hz)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Strategy {
    /// Use `native` when the monitor already has a timing at this resolution, otherwise `cvt-rb2`.
    Auto,
    /// Keep the monitor's own blanking at this resolution and raise only the pixel clock.
    /// This is what most successful monitor overclocks look like.
    Native,
    /// VESA CVT reduced blanking v1 (very widely accepted).
    CvtRb,
    /// VESA CVT reduced blanking v2 (lowest pixel clock, best for hitting bandwidth limits).
    CvtRb2,
}

impl fmt::Display for Strategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use clap::ValueEnum;
        f.write_str(self.to_possible_value().unwrap().get_name())
    }
}

/// Pixel clock in 10 kHz units for the given totals and refresh rate.
fn clock_for(h_total: u32, v_total: u32, hz: f64) -> u32 {
    (h_total as f64 * v_total as f64 * hz / 10_000.0).round() as u32
}

/// The highest-refresh existing timing at the mode's resolution, if any.
pub fn native_at(edid: &Edid, mode: &Mode) -> Option<Dtd> {
    edid.timings()
        .into_iter()
        .map(|(_, d)| d)
        .filter(|d| d.h_active == mode.width && d.v_active == mode.height && !d.interlaced())
        .max_by(|a, b| a.refresh_hz().total_cmp(&b.refresh_hz()))
}

pub fn scale_native(base: &Dtd, hz: f64) -> Dtd {
    Dtd {
        clock: clock_for(base.h_total(), base.v_total(), hz),
        ..base.clone()
    }
}

/// VESA CVT 1.2 reduced-blanking timing (v1 or v2), progressive, no margins.
pub fn cvt_rb(mode: &Mode, v2: bool) -> Result<Dtd> {
    const MIN_VBLANK_US: f64 = 460.0;
    let (w, h, hz) = (mode.width, mode.height, mode.hz);

    let h_period_us = (1_000_000.0 / hz - MIN_VBLANK_US) / h as f64;
    if h_period_us <= 0.0 {
        bail!("{hz} Hz is too fast for CVT reduced blanking at {w}x{h}");
    }
    let vbi_lines = (MIN_VBLANK_US / h_period_us).floor() as u16 + 1;

    let (h_blank, h_front, h_sync, v_front, v_sync, v_back);
    if v2 {
        (h_blank, h_front, h_sync) = (80, 8, 32);
        v_sync = 8;
        v_back = 6;
        v_front = vbi_lines.max(1 + v_sync + v_back) - v_sync - v_back;
    } else {
        (h_blank, h_front, h_sync) = (160, 48, 32);
        let ratio = w as f64 / h as f64;
        let near = |r: f64| (ratio - r).abs() < 0.01;
        v_sync = if near(4.0 / 3.0) {
            4
        } else if near(16.0 / 9.0) {
            5
        } else if near(16.0 / 10.0) {
            6
        } else if near(5.0 / 4.0) || near(15.0 / 9.0) {
            7
        } else {
            10
        };
        v_front = 3;
        v_back = vbi_lines.max(v_front + v_sync + 6) - v_front - v_sync;
    }
    let v_blank = v_front + v_sync + v_back;

    Ok(Dtd {
        clock: clock_for(w as u32 + h_blank as u32, h as u32 + v_blank as u32, hz),
        h_active: w,
        h_blank,
        h_front,
        h_sync,
        v_active: h,
        v_blank,
        v_front,
        v_sync,
        h_size_mm: 0,
        v_size_mm: 0,
        h_border: 0,
        v_border: 0,
        // Digital separate sync, +hsync, -vsync (as CVT-RB specifies).
        flags: 0x1A,
    })
}

/// Build the timing for `mode` against this monitor's EDID.
pub fn build(edid: &Edid, mode: &Mode, strategy: Strategy) -> Result<(Dtd, Strategy)> {
    let native = native_at(edid, mode);
    let strategy = match (strategy, &native) {
        (Strategy::Auto, Some(_)) => Strategy::Native,
        (Strategy::Auto, None) => Strategy::CvtRb2,
        (s, _) => s,
    };
    let mut dtd = match strategy {
        Strategy::Native => {
            let base = native.with_context(|| {
                format!(
                    "the monitor has no detailed timing at {}x{} to scale; use --timing cvt-rb or cvt-rb2",
                    mode.width, mode.height
                )
            })?;
            scale_native(&base, mode.hz)
        }
        Strategy::CvtRb => cvt_rb(mode, false)?,
        Strategy::CvtRb2 => cvt_rb(mode, true)?,
        Strategy::Auto => unreachable!(),
    };
    // Keep the physical size so desktops still compute the right DPI.
    if dtd.h_size_mm == 0
        && let Some((_, first)) = edid.timings().first()
    {
        dtd.h_size_mm = first.h_size_mm;
        dtd.v_size_mm = first.v_size_mm;
    }
    Ok((dtd, strategy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edid::tests::sample_edid;

    #[test]
    fn parse_modes() {
        let m: Mode = "2560x1440@170".parse().unwrap();
        assert_eq!((m.width, m.height, m.hz), (2560, 1440, 170.0));
        assert_eq!("1920X1080@74.97Hz".parse::<Mode>().unwrap().hz, 74.97);
        assert!("2560x1440".parse::<Mode>().is_err());
        assert!("axb@60".parse::<Mode>().is_err());
    }

    #[test]
    fn cvt_rb_matches_vesa_table() {
        // VESA DMT lists 1920x1080@60 CVT-RB as 138.5 MHz, 2080x1111.
        let d = cvt_rb(&"1920x1080@60".parse().unwrap(), false).unwrap();
        assert_eq!((d.h_total(), d.v_total()), (2080, 1111));
        assert!((d.clock_mhz() - 138.5).abs() < 0.2, "{}", d.clock_mhz());
    }

    #[test]
    fn cvt_rb2_is_tighter() {
        let m: Mode = "2560x1440@170".parse().unwrap();
        let v1 = cvt_rb(&m, false).unwrap();
        let v2 = cvt_rb(&m, true).unwrap();
        assert!(v2.clock < v1.clock);
        assert!((v2.refresh_hz() - 170.0).abs() < 0.05);
    }

    #[test]
    fn auto_prefers_native() {
        let e = Edid::parse(&sample_edid()).unwrap();
        let (d, s) = build(&e, &"2560x1440@165".parse().unwrap(), Strategy::Auto).unwrap();
        assert_eq!(s, Strategy::Native);
        assert_eq!((d.h_total(), d.v_total()), (2720, 1522));
        assert!((d.refresh_hz() - 165.0).abs() < 0.01);
        let (_, s) = build(&e, &"1920x1080@120".parse().unwrap(), Strategy::Auto).unwrap();
        assert_eq!(s, Strategy::CvtRb2);
    }
}
