//! Minimal EDID reader/writer: just enough to list detailed timings, add a new
//! one, keep the range-limits descriptor consistent and fix up checksums.

use anyhow::{Result, bail};
use std::fmt;

pub const BLOCK: usize = 128;
const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const BASE_SLOTS: [usize; 4] = [54, 72, 90, 108];
const CTA_TAG: u8 = 0x02;
const DISPLAYID_TAG: u8 = 0x70;

/// Display descriptor tags that can be overwritten without losing anything a
/// desktop or the kernel relies on, most expendable first.
const EXPENDABLE_TAGS: [u8; 7] = [0x10, 0xFE, 0xFA, 0xF7, 0xF8, 0xF9, 0xFB];

/// An 18-byte Detailed Timing Descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct Dtd {
    /// Pixel clock in units of 10 kHz.
    pub clock: u32,
    pub h_active: u16,
    pub h_blank: u16,
    pub h_front: u16,
    pub h_sync: u16,
    pub v_active: u16,
    pub v_blank: u16,
    pub v_front: u16,
    pub v_sync: u16,
    pub h_size_mm: u16,
    pub v_size_mm: u16,
    pub h_border: u8,
    pub v_border: u8,
    /// Raw byte 17: interlace, stereo and sync type/polarity.
    pub flags: u8,
}

impl Dtd {
    pub fn decode(b: &[u8]) -> Option<Dtd> {
        let clock = u16::from_le_bytes([b[0], b[1]]) as u32;
        if clock == 0 {
            return None;
        }
        let hi = |byte: u8, shift: u8| ((byte >> shift) & 0x0F) as u16;
        Some(Dtd {
            clock,
            h_active: b[2] as u16 | hi(b[4], 4) << 8,
            h_blank: b[3] as u16 | hi(b[4], 0) << 8,
            v_active: b[5] as u16 | hi(b[7], 4) << 8,
            v_blank: b[6] as u16 | hi(b[7], 0) << 8,
            h_front: b[8] as u16 | ((b[11] as u16 >> 6) & 0x3) << 8,
            h_sync: b[9] as u16 | ((b[11] as u16 >> 4) & 0x3) << 8,
            v_front: (b[10] >> 4) as u16 | ((b[11] as u16 >> 2) & 0x3) << 4,
            v_sync: (b[10] & 0x0F) as u16 | (b[11] as u16 & 0x3) << 4,
            h_size_mm: b[12] as u16 | hi(b[14], 4) << 8,
            v_size_mm: b[13] as u16 | hi(b[14], 0) << 8,
            h_border: b[15],
            v_border: b[16],
            flags: b[17],
        })
    }

    pub fn encode(&self) -> Result<[u8; 18]> {
        let check = |what: &str, value: u32, max: u32| -> Result<()> {
            if value > max {
                bail!(
                    "{what} = {value} does not fit in an EDID detailed timing (max {max}); \
                     this mode would need a DisplayID timing, which monitor-oc does not write yet"
                );
            }
            Ok(())
        };
        check("pixel clock (10 kHz units)", self.clock, 0xFFFF)?;
        check("horizontal active", self.h_active.into(), 0xFFF)?;
        check("horizontal blanking", self.h_blank.into(), 0xFFF)?;
        check("vertical active", self.v_active.into(), 0xFFF)?;
        check("vertical blanking", self.v_blank.into(), 0xFFF)?;
        check("horizontal front porch", self.h_front.into(), 0x3FF)?;
        check("horizontal sync width", self.h_sync.into(), 0x3FF)?;
        check("vertical front porch", self.v_front.into(), 0x3F)?;
        check("vertical sync width", self.v_sync.into(), 0x3F)?;
        check("image width (mm)", self.h_size_mm.into(), 0xFFF)?;
        check("image height (mm)", self.v_size_mm.into(), 0xFFF)?;
        if self.h_front + self.h_sync > self.h_blank || self.v_front + self.v_sync > self.v_blank {
            bail!("sync pulse does not fit inside the blanking interval");
        }

        let lo = |v: u16| (v & 0xFF) as u8;
        let nib = |v: u16| ((v >> 8) & 0x0F) as u8;
        let mut b = [0u8; 18];
        b[0..2].copy_from_slice(&(self.clock as u16).to_le_bytes());
        b[2] = lo(self.h_active);
        b[3] = lo(self.h_blank);
        b[4] = nib(self.h_active) << 4 | nib(self.h_blank);
        b[5] = lo(self.v_active);
        b[6] = lo(self.v_blank);
        b[7] = nib(self.v_active) << 4 | nib(self.v_blank);
        b[8] = lo(self.h_front);
        b[9] = lo(self.h_sync);
        b[10] = ((self.v_front & 0x0F) as u8) << 4 | (self.v_sync & 0x0F) as u8;
        b[11] = (((self.h_front >> 8) & 0x3) as u8) << 6
            | (((self.h_sync >> 8) & 0x3) as u8) << 4
            | (((self.v_front >> 4) & 0x3) as u8) << 2
            | ((self.v_sync >> 4) & 0x3) as u8;
        b[12] = lo(self.h_size_mm);
        b[13] = lo(self.v_size_mm);
        b[14] = nib(self.h_size_mm) << 4 | nib(self.v_size_mm);
        b[15] = self.h_border;
        b[16] = self.v_border;
        b[17] = self.flags;
        Ok(b)
    }

    pub fn h_total(&self) -> u32 {
        self.h_active as u32 + self.h_blank as u32
    }

    pub fn v_total(&self) -> u32 {
        self.v_active as u32 + self.v_blank as u32
    }

    pub fn clock_mhz(&self) -> f64 {
        self.clock as f64 / 100.0
    }

    pub fn refresh_hz(&self) -> f64 {
        self.clock as f64 * 10_000.0 / (self.h_total() as f64 * self.v_total() as f64)
    }

    pub fn h_freq_khz(&self) -> f64 {
        self.clock as f64 * 10.0 / self.h_total() as f64
    }

    pub fn interlaced(&self) -> bool {
        self.flags & 0x80 != 0
    }
}

impl fmt::Display for Dtd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{}{} @ {:.3} Hz  (pixel clock {:.2} MHz, total {}x{}, {:.2} kHz)",
            self.h_active,
            self.v_active,
            if self.interlaced() { "i" } else { "" },
            self.refresh_hz(),
            self.clock_mhz(),
            self.h_total(),
            self.v_total(),
            self.h_freq_khz(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Location {
    /// One of the four 18-byte descriptor slots in the base block (0 = preferred).
    Base(usize),
    /// A DTD inside a CTA-861 extension block.
    Cta { block: usize, offset: usize },
    /// A DisplayID Type I / Type VII detailed timing (no 655 MHz clock limit).
    DisplayId { block: usize },
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Base(0) => write!(f, "base block, preferred slot"),
            Location::Base(i) => write!(f, "base block, descriptor {}", i + 1),
            Location::Cta { block, .. } => write!(f, "CTA-861 extension (block {block})"),
            Location::DisplayId { block } => write!(f, "DisplayID extension (block {block})"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RangeLimits {
    pub min_v_hz: u16,
    pub max_v_hz: u16,
    pub min_h_khz: u16,
    pub max_h_khz: u16,
    pub max_clock_mhz: u16,
}

#[derive(Clone)]
pub struct Edid {
    pub bytes: Vec<u8>,
}

impl Edid {
    pub fn parse(raw: &[u8]) -> Result<Edid> {
        if raw.len() < BLOCK {
            bail!("EDID is {} bytes, expected at least {BLOCK}", raw.len());
        }
        if raw[..8] != HEADER {
            bail!("not an EDID (bad header)");
        }
        let blocks = 1 + raw[126] as usize;
        let len = (blocks * BLOCK).min(raw.len() / BLOCK * BLOCK);
        let edid = Edid {
            bytes: raw[..len].to_vec(),
        };
        if !edid.block_checksum_ok(0) {
            bail!("EDID base block checksum is invalid");
        }
        Ok(edid)
    }

    pub fn block_count(&self) -> usize {
        self.bytes.len() / BLOCK
    }

    fn block(&self, i: usize) -> &[u8] {
        &self.bytes[i * BLOCK..(i + 1) * BLOCK]
    }

    fn block_checksum_ok(&self, i: usize) -> bool {
        self.block(i).iter().fold(0u8, |a, b| a.wrapping_add(*b)) == 0
    }

    pub fn fix_checksums(&mut self) {
        for i in 0..self.block_count() {
            let start = i * BLOCK;
            let sum = self.bytes[start..start + BLOCK - 1]
                .iter()
                .fold(0u8, |a, b| a.wrapping_add(*b));
            self.bytes[start + BLOCK - 1] = 0u8.wrapping_sub(sum);
        }
    }

    pub fn manufacturer(&self) -> String {
        let v = u16::from_be_bytes([self.bytes[8], self.bytes[9]]);
        [10, 5, 0]
            .iter()
            .map(|s| (b'A' - 1 + ((v >> s) & 0x1F) as u8) as char)
            .collect()
    }

    pub fn product_code(&self) -> u16 {
        u16::from_le_bytes([self.bytes[10], self.bytes[11]])
    }

    fn display_descriptor(&self, slot: usize) -> Option<(u8, &[u8])> {
        let d = &self.bytes[BASE_SLOTS[slot]..BASE_SLOTS[slot] + 18];
        (d[0] == 0 && d[1] == 0).then(|| (d[3], d))
    }

    pub fn name(&self) -> Option<String> {
        (0..4).find_map(|slot| match self.display_descriptor(slot) {
            Some((0xFC, d)) => {
                let text: String = d[5..18]
                    .iter()
                    .take_while(|&&c| c != 0x0A)
                    .map(|&c| c as char)
                    .collect();
                Some(text.trim().to_string())
            }
            _ => None,
        })
    }

    /// CTA blocks as (block index, DTD start offset).
    fn cta_blocks(&self) -> Vec<(usize, usize)> {
        (1..self.block_count())
            .filter(|&i| self.block(i)[0] == CTA_TAG)
            .filter_map(|i| match self.block(i)[2] {
                0 => Some((i, 4)),
                d @ 4..=127 => Some((i, d as usize)),
                _ => None,
            })
            .collect()
    }

    pub fn timings(&self) -> Vec<(Location, Dtd)> {
        let mut out: Vec<_> = (0..4)
            .filter_map(|slot| {
                let off = BASE_SLOTS[slot];
                Dtd::decode(&self.bytes[off..off + 18]).map(|d| (Location::Base(slot), d))
            })
            .collect();
        for (block, start) in self.cta_blocks() {
            let base = block * BLOCK;
            if self.bytes[base + 2] == 0 {
                continue;
            }
            let mut off = start;
            while off + 18 < BLOCK {
                match Dtd::decode(&self.bytes[base + off..base + off + 18]) {
                    Some(d) => out.push((Location::Cta { block, offset: off }, d)),
                    None => break,
                }
                off += 18;
            }
        }
        for (block, section_end, _) in self.displayid_sections() {
            let base = block * BLOCK;
            let mut pos = 5;
            while pos + 3 <= section_end {
                let (tag, len) = (self.bytes[base + pos], self.bytes[base + pos + 2] as usize);
                let payload = base + pos + 3;
                if matches!(tag, DID_TYPE1 | DID_TYPE7) && pos + 3 + len <= section_end {
                    for t in self.bytes[payload..payload + len].chunks_exact(20) {
                        out.push((
                            Location::DisplayId { block },
                            decode_displayid(t, tag == DID_TYPE7),
                        ));
                    }
                }
                pos += 3 + len;
            }
        }
        out
    }

    /// DisplayID extension blocks as (block index, offset of the section
    /// checksum byte, DisplayID version byte). Offsets are within the block.
    fn displayid_sections(&self) -> Vec<(usize, usize, u8)> {
        (1..self.block_count())
            .filter(|&i| self.block(i)[0] == DISPLAYID_TAG)
            .filter_map(|i| {
                let b = self.block(i);
                let end = 5 + b[2] as usize;
                let sum = b[1..=end.min(126)]
                    .iter()
                    .fold(0u8, |a, x| a.wrapping_add(*x));
                (end <= 126 && sum == 0).then_some((i, end, b[1]))
            })
            .collect()
    }

    /// Add `dtd` as a DisplayID detailed timing, in an existing DisplayID
    /// section if it has room, otherwise in a new extension block.
    fn add_displayid(&mut self, dtd: &Dtd, preferred: bool) -> Result<Location> {
        let need = 3 + 20;
        let existing = self.displayid_sections().into_iter().find(|&(i, end, _)| {
            end - 5 + need <= 121
                && self.block(i)[end + 1..end + 1 + need]
                    .iter()
                    .all(|&b| b == 0)
        });
        let (block, end, version) = match existing {
            Some(found) => found,
            None => {
                if self.block_count() >= 255 {
                    bail!("EDID has no room for another extension block");
                }
                let mut blk = [0u8; BLOCK];
                blk[0] = DISPLAYID_TAG;
                blk[1] = 0x13; // DisplayID 1.3
                blk[3] = 0x03; // product type: standalone display device
                self.bytes.extend_from_slice(&blk);
                self.bytes[126] += 1;
                (self.block_count() - 1, 5, 0x13)
            }
        };
        let type7 = version >= 0x20;
        let base = block * BLOCK;
        let tag = if type7 { DID_TYPE7 } else { DID_TYPE1 };
        let timing = encode_displayid(dtd, type7, preferred)?;
        self.bytes[base + end..base + end + 3].copy_from_slice(&[tag, 0, 20]);
        self.bytes[base + end + 3..base + end + need].copy_from_slice(&timing);
        self.bytes[base + 2] += need as u8;
        let new_end = end + need;
        let sum = self.bytes[base + 1..base + new_end]
            .iter()
            .fold(0u8, |a, x| a.wrapping_add(*x));
        self.bytes[base + new_end] = 0u8.wrapping_sub(sum);
        self.fix_checksums();
        Ok(Location::DisplayId { block })
    }

    fn range_limits_offset(&self) -> Option<usize> {
        (0..4).find_map(|slot| match self.display_descriptor(slot) {
            Some((0xFD, _)) => Some(BASE_SLOTS[slot]),
            _ => None,
        })
    }

    pub fn range_limits(&self) -> Option<RangeLimits> {
        let o = self.range_limits_offset()?;
        let b = &self.bytes[o..o + 18];
        let add = |bit: u8| if b[4] & bit != 0 { 255 } else { 0 };
        Some(RangeLimits {
            min_v_hz: b[5] as u16 + add(0x01),
            max_v_hz: b[6] as u16 + add(0x02),
            min_h_khz: b[7] as u16 + add(0x04),
            max_h_khz: b[8] as u16 + add(0x08),
            max_clock_mhz: b[9] as u16 * 10,
        })
    }

    /// Raise the range-limits descriptor so it does not exclude `dtd`.
    /// Returns a human-readable list of what changed.
    pub fn widen_range_limits(&mut self, dtd: &Dtd) -> Vec<String> {
        let Some(o) = self.range_limits_offset() else {
            return Vec::new();
        };
        let Some(limits) = self.range_limits() else {
            return Vec::new();
        };
        let mut changes = Vec::new();

        let want_v = dtd.refresh_hz().ceil() as u16;
        if want_v > limits.max_v_hz {
            let (raw, offset) = if want_v > 255 {
                (want_v - 255, true)
            } else {
                (want_v, false)
            };
            if raw <= 255 && (!offset || limits.min_v_hz <= 255) {
                self.bytes[o + 6] = raw as u8;
                if offset {
                    self.bytes[o + 4] |= 0x02;
                }
                changes.push(format!(
                    "max vertical rate {} -> {want_v} Hz",
                    limits.max_v_hz
                ));
            }
        }

        let want_h = dtd.h_freq_khz().ceil() as u16;
        if want_h > limits.max_h_khz {
            let (raw, offset) = if want_h > 255 {
                (want_h - 255, true)
            } else {
                (want_h, false)
            };
            if raw <= 255 {
                self.bytes[o + 8] = raw as u8;
                if offset {
                    self.bytes[o + 4] |= 0x08;
                }
                changes.push(format!(
                    "max horizontal rate {} -> {want_h} kHz",
                    limits.max_h_khz
                ));
            }
        }

        let want_clock = (dtd.clock_mhz() / 10.0).ceil() as u16;
        if want_clock * 10 > limits.max_clock_mhz && want_clock <= 255 {
            self.bytes[o + 9] = want_clock as u8;
            changes.push(format!(
                "max pixel clock {} -> {} MHz",
                limits.max_clock_mhz,
                want_clock * 10
            ));
        }
        changes
    }

    /// A slot that can take a new DTD; never the preferred slot.
    fn free_location(&self) -> Option<Location> {
        for tag in EXPENDABLE_TAGS {
            for slot in 1..4 {
                if matches!(self.display_descriptor(slot), Some((t, _)) if t == tag) {
                    return Some(Location::Base(slot));
                }
            }
        }
        for (block, start) in self.cta_blocks() {
            let base = block * BLOCK;
            let mut off = start;
            while off + 18 < BLOCK {
                if self.bytes[base + off] == 0 && self.bytes[base + off + 1] == 0 {
                    return Some(Location::Cta { block, offset: off });
                }
                off += 18;
            }
        }
        None
    }

    fn write_at(&mut self, loc: Location, data: &[u8; 18]) {
        let start = match loc {
            Location::Base(slot) => BASE_SLOTS[slot],
            Location::Cta { block, offset } => {
                let base = block * BLOCK;
                if self.bytes[base + 2] == 0 {
                    self.bytes[base + 2] = 4;
                }
                base + offset
            }
            Location::DisplayId { .. } => unreachable!("18-byte DTDs never go into DisplayID"),
        };
        self.bytes[start..start + 18].copy_from_slice(data);
    }

    /// Add `dtd` to the EDID. With `preferred`, the new timing takes the base
    /// block's preferred slot and the previous preferred timing is moved
    /// elsewhere so it remains available. Timings too fast for an 18-byte
    /// descriptor (over 655.35 MHz), or with no descriptor slot left, go into
    /// a DisplayID extension instead.
    pub fn add_timing(&mut self, dtd: &Dtd, preferred: bool) -> Result<Location> {
        if self.timings().iter().any(|(_, t)| t == dtd) {
            bail!("the EDID already contains exactly this timing");
        }
        let Ok(encoded) = dtd.encode() else {
            return self.add_displayid(dtd, preferred);
        };
        if preferred {
            let mut old = [0u8; 18];
            old.copy_from_slice(&self.bytes[BASE_SLOTS[0]..BASE_SLOTS[0] + 18]);
            if Dtd::decode(&old).is_some() {
                let Some(dest) = self.free_location() else {
                    return self.add_displayid(dtd, preferred);
                };
                self.write_at(dest, &old);
            }
            self.write_at(Location::Base(0), &encoded);
            self.fix_checksums();
            return Ok(Location::Base(0));
        }
        let Some(dest) = self.free_location() else {
            return self.add_displayid(dtd, preferred);
        };
        self.write_at(dest, &encoded);
        self.fix_checksums();
        Ok(dest)
    }
}

const DID_TYPE1: u8 = 0x03;
const DID_TYPE7: u8 = 0x22;

/// Decode a 20-byte DisplayID Type I (10 kHz clock) or Type VII (1 kHz clock) timing.
fn decode_displayid(t: &[u8], type7: bool) -> Dtd {
    let w = |i: usize, mask: u16| (u16::from_le_bytes([t[i], t[i + 1]]) & mask) + 1;
    let raw_clock = (t[0] as u32 | (t[1] as u32) << 8 | (t[2] as u32) << 16) + 1;
    let h_pos = t[9] & 0x80 != 0;
    let v_pos = t[17] & 0x80 != 0;
    Dtd {
        clock: if type7 {
            (raw_clock + 5) / 10
        } else {
            raw_clock
        },
        h_active: w(4, 0xFFFF),
        h_blank: w(6, 0xFFFF),
        h_front: w(8, 0x7FFF),
        h_sync: w(10, 0xFFFF),
        v_active: w(12, 0xFFFF),
        v_blank: w(14, 0xFFFF),
        v_front: w(16, 0x7FFF),
        v_sync: w(18, 0xFFFF),
        h_size_mm: 0,
        v_size_mm: 0,
        h_border: 0,
        v_border: 0,
        flags: 0x18
            | (v_pos as u8) << 2
            | (h_pos as u8) << 1
            | if t[3] & 0x10 != 0 { 0x80 } else { 0 },
    }
}

/// DisplayID aspect ratio code for a timing (8 = undefined).
fn aspect_code(d: &Dtd) -> u8 {
    let ratio = d.h_active as f64 / d.v_active as f64;
    [
        (1.0, 0),
        (1.25, 1),
        (4.0 / 3.0, 2),
        (15.0 / 9.0, 3),
        (16.0 / 9.0, 4),
        (1.6, 5),
        (64.0 / 27.0, 6),
        (256.0 / 135.0, 7),
    ]
    .iter()
    .find(|(r, _)| (ratio - r).abs() < 0.01)
    .map_or(8, |&(_, code)| code)
}

fn encode_displayid(d: &Dtd, type7: bool, preferred: bool) -> Result<[u8; 20]> {
    if d.h_front + d.h_sync > d.h_blank || d.v_front + d.v_sync > d.v_blank {
        bail!("sync pulse does not fit inside the blanking interval");
    }
    if d.h_front > 0x8000
        || d.v_front > 0x8000
        || [
            d.h_active, d.h_blank, d.h_sync, d.v_active, d.v_blank, d.v_sync,
        ]
        .contains(&0)
    {
        bail!("timing cannot be represented as a DisplayID detailed timing");
    }
    let clock = if type7 { d.clock * 10 } else { d.clock } - 1;
    // Separate digital sync carries polarities; otherwise use +H -V.
    let (h_pos, v_pos) = if d.flags & 0x18 == 0x18 {
        (d.flags & 0x02 != 0, d.flags & 0x04 != 0)
    } else {
        (true, false)
    };
    let mut t = [0u8; 20];
    t[..3].copy_from_slice(&clock.to_le_bytes()[..3]);
    t[3] =
        if preferred { 0x80 } else { 0 } | if d.interlaced() { 0x10 } else { 0 } | aspect_code(d);
    let mut put = |i: usize, v: u16| t[i..i + 2].copy_from_slice(&(v - 1).to_le_bytes());
    put(4, d.h_active);
    put(6, d.h_blank);
    put(8, d.h_front);
    put(10, d.h_sync);
    put(12, d.v_active);
    put(14, d.v_blank);
    put(16, d.v_front);
    put(18, d.v_sync);
    t[9] |= (h_pos as u8) << 7;
    t[17] |= (v_pos as u8) << 7;
    Ok(t)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic 2560x1440@144 monitor with a CTA extension, built in code
    /// so the tests do not depend on any real monitor's EDID.
    pub fn sample_edid() -> Vec<u8> {
        let mut b = vec![0u8; 256];
        b[..8].copy_from_slice(&HEADER);
        b[8..10].copy_from_slice(&0x1E6Du16.to_be_bytes()); // "GSM"
        b[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
        b[18] = 1;
        b[19] = 4;
        let native = Dtd {
            clock: 59_615,
            h_active: 2560,
            h_blank: 160,
            h_front: 48,
            h_sync: 32,
            v_active: 1440,
            v_blank: 82,
            v_front: 3,
            v_sync: 5,
            h_size_mm: 597,
            v_size_mm: 336,
            h_border: 0,
            v_border: 0,
            flags: 0x1A,
        };
        b[54..72].copy_from_slice(&native.encode().unwrap());
        // Range limits: 48-144 Hz, 30-230 kHz, 600 MHz.
        b[72..90].copy_from_slice(&[
            0, 0, 0, 0xFD, 0, 48, 144, 30, 230, 60, 1, 0x0A, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        ]);
        let mut name = [0x20u8; 18];
        name[..5].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        name[5..13].copy_from_slice(b"TEST 27\n");
        b[90..108].copy_from_slice(&name);
        b[108..126].copy_from_slice(&[0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        b[126] = 1;
        b[128] = CTA_TAG;
        b[129] = 3;
        b[130] = 4;
        let mut e = Edid { bytes: b };
        e.fix_checksums();
        e.bytes
    }

    #[test]
    fn dtd_round_trip() {
        let e = Edid::parse(&sample_edid()).unwrap();
        let (_, d) = &e.timings()[0];
        let again = Dtd::decode(&d.encode().unwrap()).unwrap();
        assert_eq!(*d, again);
        assert!((d.refresh_hz() - 144.0).abs() < 0.1, "{}", d.refresh_hz());
    }

    #[test]
    fn identity() {
        let e = Edid::parse(&sample_edid()).unwrap();
        assert_eq!(e.manufacturer(), "GSM");
        assert_eq!(e.product_code(), 0x1234);
        assert_eq!(e.name().as_deref(), Some("TEST 27"));
        assert_eq!(e.range_limits().unwrap().max_v_hz, 144);
    }

    #[test]
    fn add_uses_dummy_slot_then_cta() {
        let mut e = Edid::parse(&sample_edid()).unwrap();
        let mut d = e.timings()[0].1.clone();
        d.clock += 100;
        assert_eq!(e.add_timing(&d, false).unwrap(), Location::Base(3));
        d.clock += 100;
        assert!(matches!(
            e.add_timing(&d, false).unwrap(),
            Location::Cta {
                block: 1,
                offset: 4
            }
        ));
        let reparsed = Edid::parse(&e.bytes).unwrap();
        assert!(reparsed.block_checksum_ok(1));
        assert_eq!(reparsed.timings().len(), 3);
    }

    #[test]
    fn preferred_keeps_old_native() {
        let mut e = Edid::parse(&sample_edid()).unwrap();
        let old = e.timings()[0].1.clone();
        let mut new = old.clone();
        new.clock = 62_000;
        e.add_timing(&new, true).unwrap();
        let t = Edid::parse(&e.bytes).unwrap().timings();
        assert_eq!(t[0], (Location::Base(0), new));
        assert!(t.iter().any(|(_, d)| *d == old));
    }

    #[test]
    fn fast_modes_go_to_displayid() {
        let mut e = Edid::parse(&sample_edid()).unwrap();
        let mut d = e.timings()[0].1.clone();
        d.clock = 75_000; // 750 MHz: too fast for an 18-byte DTD
        assert!(d.encode().is_err());
        assert_eq!(
            e.add_timing(&d, false).unwrap(),
            Location::DisplayId { block: 2 }
        );
        let mut d2 = d.clone();
        d2.clock = 76_000;
        assert_eq!(
            e.add_timing(&d2, true).unwrap(),
            Location::DisplayId { block: 2 }
        );

        let reparsed = Edid::parse(&e.bytes).unwrap();
        assert_eq!(reparsed.block_count(), 3);
        assert!((0..3).all(|i| reparsed.block_checksum_ok(i)));
        let found: Vec<_> = reparsed
            .timings()
            .into_iter()
            .filter(|(l, _)| matches!(l, Location::DisplayId { .. }))
            .collect();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].1.clock, d.clock);
        assert_eq!(
            (found[0].1.h_total(), found[0].1.v_total()),
            (d.h_total(), d.v_total())
        );
        assert_eq!(
            (found[0].1.h_front, found[0].1.v_sync, found[0].1.flags),
            (d.h_front, d.v_sync, d.flags)
        );
        assert_eq!(found[1].1.clock, d2.clock);
        // The DisplayID section checksum (bytes 1..=checksum) must be zero, as the kernel checks.
        let blk = &reparsed.bytes[256..384];
        let end = 5 + blk[2] as usize;
        assert_eq!(blk[1..=end].iter().fold(0u8, |a, x| a.wrapping_add(*x)), 0);
    }

    #[test]
    fn widen_limits() {
        let mut e = Edid::parse(&sample_edid()).unwrap();
        let mut d = e.timings()[0].1.clone();
        d.clock = 69_000; // ~166.7 Hz, 253.7 kHz, 690 MHz
        let changes = e.widen_range_limits(&d);
        assert_eq!(changes.len(), 3);
        let r = e.range_limits().unwrap();
        assert_eq!(r.max_v_hz, 167);
        assert_eq!(r.max_h_khz, 254);
        assert_eq!(r.max_clock_mhz, 690);
    }
}
