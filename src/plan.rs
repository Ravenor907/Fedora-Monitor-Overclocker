//! What adding a mode to a monitor's EDID will do, before anything is written.
//! Shared by the CLI (`generate`, `test`, `apply`) and the GUI's live preview.

use crate::edid::{Dtd, Edid, Location};
use crate::timing::{self, Mode, Strategy};
use anyhow::Result;
use std::fmt;

/// HDMI 2.0's TMDS limit; faster modes need HDMI 2.1 FRL on both ends.
const HDMI_TMDS_MAX_MHZ: f64 = 600.0;

pub struct Plan {
    /// The modified EDID, ready to write.
    pub edid: Edid,
    /// The new timing.
    pub dtd: Dtd,
    /// The strategy actually used (`auto` resolved).
    pub strategy: Strategy,
    /// Where the timing was placed.
    pub location: Location,
    /// Human-readable range-limit changes, e.g. "max vertical rate 144 -> 170 Hz".
    pub limit_changes: Vec<String>,
}

/// Something worth telling the user about a plan.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    /// Stored as a DisplayID timing, which some monitors ignore.
    DisplayId,
    /// Above HDMI 2.0's bandwidth.
    HdmiBandwidth { mhz: f64 },
    /// The EDID's 10 kHz clock step cannot hit the rate exactly.
    ClockStep { actual_hz: f64, requested_hz: f64 },
}

impl Note {
    /// True for notes that predict a likely failure rather than inform.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub fn is_warning(&self) -> bool {
        matches!(self, Note::HdmiBandwidth { .. })
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Note::DisplayId => write!(
                f,
                "stored as a DisplayID timing; if the monitor rejects it, also try --timing cvt-rb2."
            ),
            Note::HdmiBandwidth { mhz } => write!(
                f,
                "{mhz:.0} MHz is above HDMI 2.0's 600 MHz limit; this only works over HDMI 2.1 (FRL) on both ends."
            ),
            Note::ClockStep {
                actual_hz,
                requested_hz,
            } => write!(
                f,
                "the EDID's 10 kHz clock step gives {actual_hz:.3} Hz instead of exactly {requested_hz}."
            ),
        }
    }
}

/// Build the timing for `mode` and add it to a copy of `base`.
pub fn build(base: &Edid, mode: &Mode, strategy: Strategy, preferred: bool) -> Result<Plan> {
    let (dtd, strategy) = timing::build(base, mode, strategy)?;
    let mut edid = base.clone();
    let limit_changes = edid.widen_range_limits(&dtd);
    let location = edid.add_timing(&dtd, preferred)?;
    Ok(Plan {
        edid,
        dtd,
        strategy,
        location,
        limit_changes,
    })
}

impl Plan {
    /// Notes for this plan; `connector` (e.g. `HDMI-A-1`) enables link checks.
    pub fn notes(&self, mode: &Mode, connector: Option<&str>) -> Vec<Note> {
        let mut notes = Vec::new();
        if matches!(self.location, Location::DisplayId { .. }) && self.strategy != Strategy::CvtRb2
        {
            notes.push(Note::DisplayId);
        }
        let mhz = self.dtd.clock_mhz();
        if connector.is_some_and(|c| c.starts_with("HDMI")) && mhz > HDMI_TMDS_MAX_MHZ {
            notes.push(Note::HdmiBandwidth { mhz });
        }
        let actual_hz = self.dtd.refresh_hz();
        if (actual_hz - mode.hz).abs() > 0.05 {
            notes.push(Note::ClockStep {
                actual_hz,
                requested_hz: mode.hz,
            });
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edid::tests::sample_edid;

    fn plan(mode: &str, strategy: Strategy) -> (Mode, Plan) {
        let base = Edid::parse(&sample_edid()).unwrap();
        let mode: Mode = mode.parse().unwrap();
        let plan = build(&base, &mode, strategy, false).unwrap();
        (mode, plan)
    }

    #[test]
    fn native_overclock_widens_limits() {
        let (mode, p) = plan("2560x1440@155", Strategy::Auto);
        assert_eq!(p.strategy, Strategy::Native);
        assert_eq!(p.location, Location::Base(3));
        assert!(
            p.limit_changes
                .iter()
                .any(|c| c.starts_with("max vertical rate 144 -> 15")),
            "{:?}",
            p.limit_changes
        );
        assert!(p.notes(&mode, Some("DP-1")).is_empty());
    }

    #[test]
    fn fast_modes_note_displayid_and_hdmi() {
        let (mode, p) = plan("2560x1440@180", Strategy::Native);
        assert!(matches!(p.location, Location::DisplayId { .. }));
        let notes = p.notes(&mode, Some("HDMI-A-1"));
        assert!(notes.contains(&Note::DisplayId));
        assert!(notes.iter().any(|n| n.is_warning()));
        assert!(!p.notes(&mode, Some("DP-1")).iter().any(Note::is_warning));
    }

    #[test]
    fn clock_step_rounding_is_reported() {
        // Tiny modes have so few pixels that one 10 kHz step is ~0.3 Hz.
        let (mode, p) = plan("200x100@120", Strategy::CvtRb2);
        let notes = p.notes(&mode, None);
        assert_eq!(notes.len(), 1, "{notes:?}");
        let text = notes[0].to_string();
        assert!(text.contains("119.876 Hz instead of exactly 120"), "{text}");
    }
}
