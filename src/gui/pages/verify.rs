//! Verify: does the screen really run at the new rate? Measures how often
//! this window is presented (vsync paces it to the display) and shows motion
//! and frame-skipping tests.

use super::{View, columns, header, row};
use crate::gamescope::TrialState;
use crate::gui::theme::{TOUCH, bold, font, palette, tint};
use crate::gui::widgets::{
    Button, Kind, banner, big_number, caption, caption_row, card, chip, dim, faint, segmented,
};
use eframe::egui::{
    self, Color32, CornerRadius, Mesh, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2,
};
use std::collections::VecDeque;
use std::time::Instant;

/// Intervals kept for the statistics (about 2.5 s at 144 Hz).
const WINDOW: usize = 360;
/// Intervals drawn in the graph.
const GRAPH: usize = 240;
/// An interval this many times the median means a frame was skipped.
const DROP_FACTOR: f64 = 1.5;
/// A rate within this fraction of the target counts as a match.
const MATCH_TOLERANCE: f64 = 0.01;
const SKIP_CELLS: usize = 36;

/// Rolling frame-interval statistics.
#[derive(Default)]
pub struct Meter {
    intervals: VecDeque<f64>,
    pub dropped: u32,
    pub frames: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    /// Refresh rate from the mean interval, skipped frames left out.
    pub hz: f64,
    pub median_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    /// Standard deviation of the intervals.
    pub jitter_ms: f64,
}

impl Meter {
    fn median(&self) -> Option<f64> {
        if self.intervals.len() < 10 {
            return None;
        }
        let mut sorted: Vec<f64> = self.intervals.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        Some(sorted[sorted.len() / 2])
    }

    /// Record the time between two presented frames, in seconds.
    pub fn push(&mut self, dt: f64) {
        if !(dt > 0.0 && dt < 0.5) {
            return;
        }
        if self.median().is_some_and(|m| dt > m * DROP_FACTOR) {
            self.dropped += 1;
        }
        self.frames += 1;
        self.intervals.push_back(dt);
        while self.intervals.len() > WINDOW {
            self.intervals.pop_front();
        }
    }

    pub fn reset(&mut self) {
        *self = Meter::default();
    }

    pub fn stats(&self) -> Option<Stats> {
        let median = self.median()?;
        let kept: Vec<f64> = self
            .intervals
            .iter()
            .copied()
            .filter(|dt| *dt <= median * DROP_FACTOR)
            .collect();
        let mean = kept.iter().sum::<f64>() / kept.len() as f64;
        let var = kept.iter().map(|dt| (dt - mean).powi(2)).sum::<f64>() / kept.len() as f64;
        let (min, max) = self
            .intervals
            .iter()
            .fold((f64::MAX, 0f64), |(lo, hi), dt| (lo.min(*dt), hi.max(*dt)));
        Some(Stats {
            hz: 1.0 / mean,
            median_ms: median * 1000.0,
            min_ms: min * 1000.0,
            max_ms: max * 1000.0,
            jitter_ms: var.sqrt() * 1000.0,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum Test {
    #[default]
    Motion,
    Skipping,
}

pub struct State {
    pub target: u32,
    meter: Meter,
    last: Option<(Instant, u64)>,
    test: Test,
    /// Frames drawn while Verify was visible (drives the frame-skip cells).
    tick: u64,
    noise: u64,
}

impl Default for State {
    fn default() -> State {
        State {
            target: 0,
            meter: Meter::default(),
            last: None,
            test: Test::Motion,
            tick: 0,
            noise: 0x9E37_79B9_7F4A_7C15,
        }
    }
}

impl State {
    /// Start over with a new target (from a trial or a tested mode).
    pub fn retarget(&mut self, hz: u32) {
        self.target = hz;
        self.meter.reset();
    }

    /// Measure one frame. Gaps (page hidden in between) are skipped.
    fn sample(&mut self, frame_nr: u64, simulated_hz: Option<f64>) {
        let now = Instant::now();
        if let Some((then, nr)) = self.last
            && nr + 1 == frame_nr
        {
            let dt = match simulated_hz {
                Some(hz) => self.simulated_interval(hz),
                None => now.duration_since(then).as_secs_f64(),
            };
            self.meter.push(dt);
        }
        self.last = Some((now, frame_nr));
        self.tick += 1;
    }

    /// Demo only: a steady display with a little timing noise.
    fn simulated_interval(&mut self, hz: f64) -> f64 {
        let mut jitter = 0.0;
        for _ in 0..3 {
            self.noise ^= self.noise << 13;
            self.noise ^= self.noise >> 7;
            self.noise ^= self.noise << 17;
            jitter += (self.noise % 1000) as f64 / 1000.0 - 0.5;
        }
        1.0 / hz + jitter * 0.000_12
    }
}

/// The rate the demo pretends the screen runs at.
fn demo_rate(view: &View) -> f64 {
    let Ok(info) = &view.snap.panel else {
        return 144.0;
    };
    let hz = match &info.settings {
        Ok(s) => match s.state.trial {
            Some((hz, TrialState::Running)) => hz,
            _ => s.state.max.max(info.stock_max),
        },
        Err(_) => info.stock_max,
    };
    hz as f64
}

/// Target before the user picks one: the running trial, the kept limit,
/// or the stock maximum.
fn default_target(view: &View) -> u32 {
    match &view.snap.panel {
        Ok(info) => match &info.settings {
            Ok(s) => match s.state.trial {
                Some((hz, TrialState::Running)) => hz,
                _ => s.state.max.max(info.stock_max),
            },
            Err(_) => info.stock_max,
        },
        Err(_) => 60,
    }
}

pub fn show(ui: &mut Ui, st: &mut State, view: &View) {
    let ctx = ui.ctx().clone();
    let simulated = view.demo.then(|| demo_rate(view));
    st.sample(ctx.cumulative_frame_nr(), simulated);
    // Draw continuously: every presented frame is a measurement.
    ctx.request_repaint();
    if st.target == 0 {
        st.target = default_target(view);
    }
    let p = palette(&ctx);

    header(
        ui,
        "Verify",
        "Measures how often this window is really shown, and checks for skipped frames.",
    );
    banner(ui, p.accent, |ui| {
        ui.label(RichText::new("Before you start").font(bold(17.0)));
        dim(
            ui,
            "Show this window on the screen you are testing. In Game Mode, turn the frame limit off \
             (Quick Access > Performance > Frame Limit) and pick the refresh rate you want to check.",
        );
        if simulated.is_some() {
            faint(ui, "Demo: the measurement is simulated.");
        }
    });

    let stats = st.meter.stats();
    columns(ui, 0.5, |ui, col| match col {
        0 => measured_card(ui, st, stats),
        _ => stats_card(ui, st, stats),
    });
    graph_card(ui, st, stats);
    tests_card(ui, st);
}

fn measured_card(ui: &mut Ui, st: &mut State, stats: Option<Stats>) {
    let p = palette(ui.ctx());
    card(ui, |ui| {
        caption(ui, "Measured", p.text_faint);
        match stats {
            Some(s) => big_number(ui, &format!("{:.1}", s.hz), "Hz", p.text, 76.0),
            None => big_number(ui, "–", "Hz", p.text_faint, 76.0),
        };
        let off = stats.map(|s| (s.hz - st.target as f64) / st.target as f64);
        match off {
            None => chip(ui, "Measuring…", p.text_dim),
            Some(off) if off.abs() <= MATCH_TOLERANCE => {
                chip(ui, &format!("✓  Matches {} Hz", st.target), p.good)
            }
            Some(off) => chip(
                ui,
                &format!("{:+.1}% from {} Hz", off * 100.0, st.target),
                p.warn,
            ),
        };
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Target").color(p.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        Button::new("+", Kind::Secondary)
                            .min_width(TOUCH)
                            .text_size(22.0),
                    )
                    .clicked()
                {
                    st.retarget(st.target + 1);
                }
                ui.label(
                    RichText::new(format!("{} Hz", st.target))
                        .font(bold(24.0))
                        .color(p.text),
                );
                if ui
                    .add_enabled(
                        st.target > 1,
                        Button::new("−", Kind::Secondary)
                            .min_width(TOUCH)
                            .text_size(22.0),
                    )
                    .clicked()
                {
                    st.retarget(st.target - 1);
                }
            });
        });
    });
}

fn stat(ui: &mut Ui, label: &str, value: String, color: Color32) {
    let p = palette(ui.ctx());
    ui.vertical(|ui| {
        caption(ui, label, p.text_faint);
        ui.label(RichText::new(value).font(bold(24.0)).color(color));
    });
}

fn stats_card(ui: &mut Ui, st: &mut State, stats: Option<Stats>) {
    let p = palette(ui.ctx());
    let fmt = |v: Option<f64>, unit: &str| v.map_or("–".into(), |v| format!("{v:.2} {unit}"));
    card(ui, |ui| {
        egui::Grid::new("verify-stats")
            .num_columns(2)
            .spacing(vec2(28.0, 18.0))
            .min_col_width((ui.available_width() - 28.0) / 2.0)
            .show(ui, |ui| {
                stat(
                    ui,
                    "Frame time",
                    fmt(stats.map(|s| s.median_ms), "ms"),
                    p.text,
                );
                stat(
                    ui,
                    "Jitter",
                    stats.map_or("–".into(), |s| format!("±{:.2} ms", s.jitter_ms)),
                    p.text,
                );
                ui.end_row();
                stat(
                    ui,
                    "Fastest / slowest",
                    stats.map_or("–".into(), |s| {
                        format!("{:.2} / {:.2} ms", s.min_ms, s.max_ms)
                    }),
                    p.text,
                );
                stat(
                    ui,
                    "Skipped frames",
                    st.meter.dropped.to_string(),
                    if st.meter.dropped == 0 {
                        p.good
                    } else {
                        p.warn
                    },
                );
                ui.end_row();
            });
        ui.horizontal(|ui| {
            faint(ui, format!("{} frames measured", st.meter.frames));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(Button::new("Reset", Kind::Ghost).height(40.0))
                    .clicked()
                {
                    st.meter.reset();
                }
            });
        });
    });
}

fn graph_card(ui: &mut Ui, st: &State, stats: Option<Stats>) {
    let p = palette(ui.ctx());
    card(ui, |ui| {
        caption_row(ui, "Frame times", |ui| {
            ui.label(
                RichText::new(format!("target {:.2} ms", 1000.0 / st.target.max(1) as f64))
                    .font(font(13.0))
                    .color(p.text_faint),
            );
        });
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(10), p.bg);
        let area = rect.shrink2(vec2(10.0, 10.0));
        let target = 1.0 / st.target.max(1) as f64;
        let top = target * 2.0;
        let y = |dt: f64| area.max.y - (dt / top).min(1.0) as f32 * area.height();
        let median = stats.map_or(target, |s| s.median_ms / 1000.0);
        let step = area.width() / (GRAPH - 1) as f32;
        let n = st.meter.intervals.len();
        let recent: Vec<f64> = st
            .meter
            .intervals
            .iter()
            .skip(n.saturating_sub(GRAPH))
            .copied()
            .collect();
        // Newest on the right.
        let x0 = area.max.x - (recent.len().saturating_sub(1)) as f32 * step;
        let points: Vec<_> = recent
            .iter()
            .enumerate()
            .map(|(i, dt)| pos2(x0 + i as f32 * step, y(*dt)))
            .collect();
        let ty = y(target);
        painter.add(Shape::dashed_line(
            &[pos2(area.min.x, ty), pos2(area.max.x, ty)],
            Stroke::new(1.0, tint(p.text_faint, 160)),
            6.0,
            5.0,
        ));
        if points.len() > 1 {
            // Area under the line, fading out downwards.
            let mut mesh = Mesh::default();
            for (i, pt) in points.iter().enumerate() {
                mesh.colored_vertex(*pt, tint(p.accent, 70));
                mesh.colored_vertex(pos2(pt.x, area.max.y), tint(p.accent, 0));
                if i > 0 {
                    let b = (i * 2) as u32;
                    mesh.add_triangle(b - 2, b - 1, b);
                    mesh.add_triangle(b - 1, b + 1, b);
                }
            }
            painter.add(mesh);
            painter.add(Shape::line(points.clone(), Stroke::new(2.0, p.accent)));
        }
        for (pt, dt) in points.iter().zip(&recent) {
            if *dt > median * DROP_FACTOR {
                painter.circle_filled(*pt, 4.0, p.bad);
            }
        }
        faint(
            ui,
            "The line should stay flat on the dashed target. Red dots are skipped frames (1.5× longer than usual).",
        );
    });
}

fn tests_card(ui: &mut Ui, st: &mut State) {
    let p = palette(ui.ctx());
    card(ui, |ui| {
        caption(ui, "Visual tests", p.text_faint);
        segmented(
            ui,
            &mut st.test,
            &[(Test::Motion, "Motion"), (Test::Skipping, "Frame skipping")],
        );
        match st.test {
            Test::Motion => motion(ui),
            Test::Skipping => skipping(ui, st.tick),
        }
    });
}

/// A block crossing the track at constant speed: it should glide; jumps or
/// stutter mean uneven or skipped frames.
fn motion(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 110.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), p.bg);
    let mut x = rect.min.x + 40.0;
    while x < rect.max.x {
        painter.line_segment(
            [pos2(x, rect.min.y + 10.0), pos2(x, rect.max.y - 10.0)],
            Stroke::new(1.0, p.border),
        );
        x += 40.0;
    }
    let size = 56.0;
    let travel = rect.width() - size - 20.0;
    let speed = 640.0; // points per second
    let t = ui.input(|i| i.time) as f32 * speed % (travel * 2.0);
    let offset = if t < travel { t } else { travel * 2.0 - t };
    let block = Rect::from_min_size(
        pos2(rect.min.x + 10.0 + offset, rect.center().y - size / 2.0),
        vec2(size, size),
    );
    painter.rect_filled(
        block.expand(6.0),
        CornerRadius::same(18),
        tint(p.accent, 40),
    );
    painter.rect_filled(block, CornerRadius::same(14), p.accent);
    faint(
        ui,
        "Follow the block with your eyes: it should glide smoothly. Regular stutter or double images \
         mean the screen is skipping or repeating frames.",
    );
}

/// One cell lights per frame. A photo with a ~1/10 s exposure must show one
/// unbroken run of lit cells; gaps are skipped frames.
fn skipping(ui: &mut Ui, tick: u64) {
    let p = palette(ui.ctx());
    let gap = 4.0;
    let width = ui.available_width();
    let cell = (width - gap * (SKIP_CELLS as f32 - 1.0)) / SKIP_CELLS as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, cell + 26.0), Sense::hover());
    let painter = ui.painter();
    let lit = (tick % SKIP_CELLS as u64) as usize;
    for i in 0..SKIP_CELLS {
        let x = rect.min.x + i as f32 * (cell + gap);
        let r = Rect::from_min_size(pos2(x, rect.min.y), vec2(cell, cell));
        let fill = if i == lit { p.accent } else { p.raised };
        painter.rect_filled(r, CornerRadius::same(4), fill);
        if i % 6 == 0 {
            painter.text(
                pos2(r.center().x, r.max.y + 6.0),
                egui::Align2::CENTER_TOP,
                (i + 1).to_string(),
                font(12.0),
                p.text_faint,
            );
        }
    }
    row(ui, |ui| {
        faint(
            ui,
            "Take a photo with about 1/10 s exposure (or film in slow motion). The lit cells must form \
             one unbroken run; gaps mean the screen skipped frames even though it reports the new rate.",
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_frames_measure_exactly() {
        let mut m = Meter::default();
        for _ in 0..200 {
            m.push(1.0 / 135.0);
        }
        let s = m.stats().unwrap();
        assert!((s.hz - 135.0).abs() < 0.01, "{s:?}");
        assert!(s.jitter_ms < 1e-6);
        assert_eq!(m.dropped, 0);
    }

    #[test]
    fn skipped_frames_are_counted_and_ignored() {
        let mut m = Meter::default();
        for i in 0..300 {
            m.push(if i % 50 == 49 {
                2.0 / 120.0
            } else {
                1.0 / 120.0
            });
        }
        assert_eq!(m.dropped, 6);
        let s = m.stats().unwrap();
        assert!((s.hz - 120.0).abs() < 0.01, "{s:?}");
        assert!((s.max_ms - 2000.0 / 120.0).abs() < 1e-6);
        // Pauses (page hidden, window moved) are not frames.
        m.push(2.0);
        assert_eq!(m.frames, 300);
    }
}
