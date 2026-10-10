//! PROTOTYPE, throwaway. Toolkit-agnostic UI core for the toolkit spike (#102).
//!
//! Both spike apps (gpui, Slint) drive this crate and only draw what it says.
//! Nothing here writes to a GPU: "Apply" only moves staged values into the
//! simulated applied state.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::Duration;

pub mod theme;

pub const HISTORY_SECS: usize = 3600;
pub const RANGES: [(usize, &str); 4] = [(60, "1m"), (300, "5m"), (900, "15m"), (3600, "1h")];
pub const CURVE_TEMP: (f32, f32) = (20.0, 100.0);
pub const CURVE_FAN: (f32, f32) = (0.0, 100.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Metric {
    Temp,
    Util,
    Power,
    Gclk,
    Mclk,
    Fan,
}

impl Metric {
    pub const ALL: [Metric; 6] = [Metric::Temp, Metric::Util, Metric::Power, Metric::Gclk, Metric::Mclk, Metric::Fan];
    fn idx(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub temp: f32,
    pub util: f32,
    pub power: f32,
    pub gclk: f32,
    pub mclk: f32,
    pub fan: f32,
    pub fan_rpm: Option<u32>,
}

impl Sample {
    pub fn get(&self, m: Metric) -> f32 {
        match m {
            Metric::Temp => self.temp,
            Metric::Util => self.util,
            Metric::Power => self.power,
            Metric::Gclk => self.gclk,
            Metric::Mclk => self.mclk,
            Metric::Fan => self.fan,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FanMode {
    Auto,
    Fixed,
    Curve,
}

impl FanMode {
    pub const ALL: [FanMode; 3] = [FanMode::Auto, FanMode::Fixed, FanMode::Curve];
    pub fn label(self) -> &'static str {
        match self {
            FanMode::Auto => "AUTO",
            FanMode::Fixed => "FIXED",
            FanMode::Curve => "CURVE",
        }
    }
}

/// A fan curve point: (temperature °C, fan %).
pub type CurvePoint = (f32, f32);

#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    pub power_limit: u32,
    pub g_offset: i32,
    pub m_offset: i32,
    pub fan_mode: FanMode,
    pub fan_fixed: u32,
    pub curve: Vec<CurvePoint>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub tuning: Tuning,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub power_min: u32,
    pub power_max: u32,
    pub max_clock: f32,
    pub max_mem_clock: f32,
}

pub struct History {
    series: [VecDeque<f32>; 6],
}

impl History {
    fn new() -> Self {
        Self { series: std::array::from_fn(|_| VecDeque::with_capacity(HISTORY_SECS)) }
    }

    fn push(&mut self, s: &Sample) {
        for m in Metric::ALL {
            let q = &mut self.series[m.idx()];
            if q.len() == HISTORY_SECS {
                q.pop_front();
            }
            q.push_back(s.get(m));
        }
    }

    pub fn len(&self) -> usize {
        self.series[0].len()
    }

    /// The last `range` seconds, averaged down to at most `cols` points.
    /// Fewer samples than `range` yields fewer points: views right-align them.
    pub fn sample(&self, m: Metric, range: usize, cols: usize) -> Vec<f32> {
        let q = &self.series[m.idx()];
        let n = range.min(q.len());
        let start = q.len() - n;
        let cols = cols.max(2);
        if n <= cols {
            return q.range(start..).copied().collect();
        }
        let step = n as f32 / cols as f32;
        (0..cols)
            .map(|i| {
                let a = start + (i as f32 * step) as usize;
                let b = (start + ((i + 1) as f32 * step) as usize).max(a + 1);
                let (sum, cnt) = q.range(a..b).fold((0.0, 0), |(s, c), v| (s + v, c + 1));
                sum / cnt as f32
            })
            .collect()
    }

    /// (max, avg, min) over the last `range` seconds.
    pub fn stats(&self, m: Metric, range: usize) -> Option<(f32, f32, f32)> {
        let q = &self.series[m.idx()];
        let n = range.min(q.len());
        if n == 0 {
            return None;
        }
        let it = q.range(q.len() - n..);
        let (mx, mn, sum) = it.fold((f32::MIN, f32::MAX, 0.0), |(mx, mn, s), &v| (mx.max(v), mn.min(v), s + v));
        Some((mx, sum / n as f32, mn))
    }
}

pub struct Gpu {
    pub short: String,
    pub bus: String,
    pub sub: String,
    pub limits: Limits,
    pub applied: Tuning,
    pub staged: Tuning,
    pub profiles: Vec<Profile>,
    pub profile: usize,
    pub startup: usize,
    pub now: Sample,
    pub history: History,
    pub cooling_paused: bool,
    pub real: bool,
    sim: Option<SimParams>,
}

impl Gpu {
    pub fn pending_count(&self) -> usize {
        let (a, s) = (&self.applied, &self.staged);
        [
            a.power_limit != s.power_limit,
            a.g_offset != s.g_offset,
            a.m_offset != s.m_offset,
            a.fan_mode != s.fan_mode,
            a.fan_fixed != s.fan_fixed,
            a.curve != s.curve,
        ]
        .iter()
        .filter(|b| **b)
        .count()
    }

    pub fn power_pending(&self) -> Option<u32> {
        (self.staged.power_limit != self.applied.power_limit).then_some(self.staged.power_limit)
    }

    pub fn curve_pending(&self) -> bool {
        self.staged.curve != self.applied.curve
    }

    pub fn modified_from_profile(&self) -> bool {
        self.profiles[self.profile].tuning != self.applied
    }

    /// Value range a chart uses for a metric on this GPU.
    pub fn scale(&self, m: Metric) -> (f32, f32) {
        match m {
            Metric::Temp => (20.0, 100.0),
            Metric::Util | Metric::Fan => (0.0, 100.0),
            Metric::Power => (0.0, self.limits.power_max as f32),
            Metric::Gclk => (0.0, self.limits.max_clock + 400.0),
            Metric::Mclk => (0.0, self.limits.max_mem_clock * 1.1),
        }
    }

    pub fn curve_target(&self) -> f32 {
        curve_at(&self.staged.curve, self.now.temp)
    }
}

pub fn curve_at(c: &[CurvePoint], temp: f32) -> f32 {
    if c.is_empty() {
        return 0.0;
    }
    if temp <= c[0].0 {
        return c[0].1;
    }
    for w in c.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if temp <= x1 {
            return y0 + (y1 - y0) * (temp - x0) / (x1 - x0);
        }
    }
    c[c.len() - 1].1
}

/// Where a value sits in a chart, as a fraction of its height from the top (0 = top).
pub fn y_frac(v: f32, (min, max): (f32, f32)) -> f32 {
    1.0 - (v.clamp(min, max) - min) / (max - min)
}

pub fn signed_mhz(v: i32) -> String {
    match v {
        0 => "±0 MHz".into(),
        v if v > 0 => format!("+{v} MHz"),
        v => format!("−{} MHz", -v),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePref {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    PowerLimit,
    GOffset,
    MOffset,
}

pub struct Command {
    pub label: String,
    pub action: Action,
}

#[derive(Clone, Debug)]
pub enum Action {
    ApplyProfile(usize),
    SelectGpu(usize),
    Apply,
    Discard,
    ToggleCurve,
    SetTheme(ThemePref),
    SetRange(usize),
}

pub struct App {
    pub gpus: Vec<Gpu>,
    pub sel: usize,
    pub range: usize,
    pub curve_open: bool,
    pub theme_pref: ThemePref,
    pub system_dark: bool,
    pub ticks: u64,
    pub source_label: &'static str,
    rx: Option<mpsc::Receiver<Vec<Sample>>>,
}

impl App {
    /// `--sim` (or no NVIDIA GPU): two simulated GPUs, as in the HTML prototype.
    /// Otherwise: real NVML readings, read-only.
    pub fn new(force_sim: bool) -> Self {
        let real = if force_sim { None } else { nvml::start() };
        let (gpus, rx, source_label) = match real {
            Some((gpus, rx)) => (gpus, Some(rx), "NVML"),
            None => (sim::gpus(), None, "SIM"),
        };
        Self {
            gpus,
            sel: 0,
            range: 300,
            curve_open: true,
            theme_pref: ThemePref::System,
            system_dark: false,
            ticks: 0,
            source_label,
            rx,
        }
    }

    /// `--sim` for simulated GPUs; `--light` / `--dark` override the system theme.
    pub fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut app = Self::new(args.iter().any(|a| a == "--sim"));
        if args.iter().any(|a| a == "--light") {
            app.theme_pref = ThemePref::Light;
        } else if args.iter().any(|a| a == "--dark") {
            app.theme_pref = ThemePref::Dark;
        }
        app
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpus[self.sel]
    }

    fn gpu_mut(&mut self) -> &mut Gpu {
        &mut self.gpus[self.sel]
    }

    pub fn dark(&self) -> bool {
        match self.theme_pref {
            ThemePref::System => self.system_dark,
            ThemePref::Light => false,
            ThemePref::Dark => true,
        }
    }

    pub fn tokens(&self) -> &'static theme::Tokens {
        theme::tokens(self.dark())
    }

    /// Called once per second by the view layer.
    pub fn tick(&mut self) {
        self.ticks += 1;
        if let Some(rx) = &self.rx {
            let mut latest = None;
            while let Ok(s) = rx.try_recv() {
                latest = Some(s);
            }
            if let Some(samples) = latest {
                for (g, s) in self.gpus.iter_mut().zip(samples) {
                    g.now = s;
                    g.history.push(&s);
                }
            }
        }
        let t = self.ticks + HISTORY_SECS as u64;
        for g in &mut self.gpus {
            if g.sim.is_some() {
                sim::step(g, t);
            }
        }
    }

    pub fn select(&mut self, i: usize) {
        if i < self.gpus.len() {
            self.sel = i;
        }
    }

    pub fn step(&mut self, what: Step, dir: i32) {
        let g = self.gpu_mut();
        match what {
            Step::PowerLimit => {
                let v = g.staged.power_limit as i32 + dir * 5;
                g.staged.power_limit = (v.max(0) as u32).clamp(g.limits.power_min, g.limits.power_max);
            }
            Step::GOffset => g.staged.g_offset = (g.staged.g_offset + dir * 15).clamp(-500, 500),
            Step::MOffset => g.staged.m_offset = (g.staged.m_offset + dir * 100).clamp(-1000, 3000),
        }
    }

    /// Power-limit drag: `y_frac` is the pointer's height in the power chart, 0 = top.
    pub fn drag_power_limit(&mut self, y_frac: f32) {
        let g = self.gpu_mut();
        let w = g.limits.power_max as f32 * (1.0 - y_frac.clamp(0.0, 1.0));
        let w = w.clamp(g.limits.power_min as f32, g.limits.power_max as f32);
        g.staged.power_limit = ((w / 5.0).round() * 5.0) as u32;
    }

    pub fn set_power_limit(&mut self, w: u32) -> bool {
        let g = self.gpu_mut();
        if !(g.limits.power_min..=g.limits.power_max).contains(&w) {
            return false;
        }
        g.staged.power_limit = w;
        true
    }

    pub fn set_fan_mode(&mut self, m: FanMode) {
        self.gpu_mut().staged.fan_mode = m;
    }

    /// Index of the curve point within `radius` (in the same normalized
    /// units as the point coordinates) of the pointer, if any.
    pub fn curve_hit(&self, nx: f32, ny: f32, aspect: f32, radius: f32) -> Option<usize> {
        let c = &self.gpu().staged.curve;
        let mut best = None;
        let mut bd = radius;
        for (i, p) in c.iter().enumerate() {
            let (px, py) = curve_to_norm(*p);
            let d = (((px - nx) * aspect).powi(2) + (py - ny).powi(2)).sqrt();
            if d < bd {
                bd = d;
                best = Some(i);
            }
        }
        best
    }

    /// Move curve point `i` to the normalized position (0..1, 0 = top), keeping
    /// points in temperature order with a 2 °C gap.
    pub fn drag_curve_point(&mut self, i: usize, nx: f32, ny: f32) {
        let g = self.gpu_mut();
        let c = &mut g.staged.curve;
        if i >= c.len() {
            return;
        }
        let (t, f) = norm_to_curve(nx, ny);
        let lo = if i > 0 { c[i - 1].0 + 2.0 } else { CURVE_TEMP.0 };
        let hi = if i + 1 < c.len() { c[i + 1].0 - 2.0 } else { CURVE_TEMP.1 };
        c[i] = (t.clamp(lo, hi).round(), f.clamp(0.0, 100.0).round());
    }

    pub fn apply(&mut self) {
        let g = self.gpu_mut();
        g.applied = g.staged.clone();
    }

    pub fn discard(&mut self) {
        let g = self.gpu_mut();
        g.staged = g.applied.clone();
    }

    pub fn apply_profile(&mut self, i: usize) {
        let g = self.gpu_mut();
        if let Some(p) = g.profiles.get(i) {
            g.profile = i;
            g.applied = p.tuning.clone();
            g.staged = p.tuning.clone();
        }
    }

    pub fn run(&mut self, a: Action) {
        match a {
            Action::ApplyProfile(i) => self.apply_profile(i),
            Action::SelectGpu(i) => self.select(i),
            Action::Apply => self.apply(),
            Action::Discard => self.discard(),
            Action::ToggleCurve => self.curve_open = !self.curve_open,
            Action::SetTheme(t) => self.theme_pref = t,
            Action::SetRange(r) => self.range = r,
        }
    }

    pub fn cycle_theme(&mut self) {
        self.theme_pref = match self.theme_pref {
            ThemePref::System => ThemePref::Light,
            ThemePref::Light => ThemePref::Dark,
            ThemePref::Dark => ThemePref::System,
        };
    }

    pub fn theme_label(&self) -> &'static str {
        match self.theme_pref {
            ThemePref::System => "THEME: SYSTEM",
            ThemePref::Light => "THEME: LIGHT",
            ThemePref::Dark => "THEME: DARK",
        }
    }

    pub fn commands(&self, query: &str) -> Vec<Command> {
        let q = query.to_lowercase();
        let g = self.gpu();
        let mut c: Vec<Command> = g
            .profiles
            .iter()
            .enumerate()
            .map(|(i, p)| Command { label: format!("Apply profile {}", p.name), action: Action::ApplyProfile(i) })
            .collect();
        for (i, x) in self.gpus.iter().enumerate() {
            c.push(Command { label: format!("Go to {}", x.short), action: Action::SelectGpu(i) });
        }
        c.push(Command { label: "Apply changes".into(), action: Action::Apply });
        c.push(Command { label: "Discard changes".into(), action: Action::Discard });
        c.push(Command { label: "Toggle fan curve".into(), action: Action::ToggleCurve });
        for (t, l) in [(ThemePref::System, "System"), (ThemePref::Light, "Light"), (ThemePref::Dark, "Dark")] {
            c.push(Command { label: format!("Theme: {l}"), action: Action::SetTheme(t) });
        }
        for (r, l) in RANGES {
            c.push(Command { label: format!("Show last {l}"), action: Action::SetRange(r) });
        }
        c.retain(|c| c.label.to_lowercase().contains(&q));
        c
    }

    pub fn status_daemon(&self) -> String {
        format!("PRISMD 0.9.2 CONNECTED · DATA: {}", self.source_label)
    }

    pub fn problems(&self) -> usize {
        self.gpus.iter().filter(|g| g.cooling_paused).count()
    }
}

/// Curve point → normalized chart position (0..1, y 0 = top).
pub fn curve_to_norm((t, f): CurvePoint) -> (f32, f32) {
    ((t - CURVE_TEMP.0) / (CURVE_TEMP.1 - CURVE_TEMP.0), 1.0 - (f - CURVE_FAN.0) / (CURVE_FAN.1 - CURVE_FAN.0))
}

pub fn norm_to_curve(nx: f32, ny: f32) -> CurvePoint {
    (CURVE_TEMP.0 + nx * (CURVE_TEMP.1 - CURVE_TEMP.0), CURVE_FAN.0 + (1.0 - ny) * (CURVE_FAN.1 - CURVE_FAN.0))
}

fn curve_quiet() -> Vec<CurvePoint> {
    vec![(35., 25.), (55., 32.), (70., 48.), (82., 72.), (90., 100.)]
}
fn curve_bal() -> Vec<CurvePoint> {
    vec![(35., 30.), (55., 40.), (68., 58.), (80., 80.), (88., 100.)]
}
fn curve_perf() -> Vec<CurvePoint> {
    vec![(30., 35.), (50., 50.), (62., 68.), (74., 88.), (84., 100.)]
}

fn profiles(power: [u32; 3], perf_offsets: (i32, i32)) -> Vec<Profile> {
    let t = |pl, g, m, curve| Tuning { power_limit: pl, g_offset: g, m_offset: m, fan_mode: FanMode::Curve, fan_fixed: 50, curve };
    vec![
        Profile { name: "QUIET".into(), tuning: t(power[0], 0, 0, curve_quiet()) },
        Profile { name: "BALANCED".into(), tuning: t(power[1], 0, 0, curve_bal()) },
        Profile { name: "PERFORMANCE".into(), tuning: t(power[2], perf_offsets.0, perf_offsets.1, curve_perf()) },
    ]
}

struct SimParams {
    game: bool,
    ambient: f32,
    thermal: f32,
    mem_clock: f32,
    seed: u64,
}

mod sim {
    use super::*;

    fn noise(seed: &mut u64, a: f32) -> f32 {
        // xorshift: deterministic, no rand dependency
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        ((*seed % 10_000) as f32 / 10_000.0 - 0.5) * 2.0 * a
    }

    pub fn gpus() -> Vec<Gpu> {
        let mk = |short: &str, bus: &str, sub: &str, limits: Limits, profiles: Vec<Profile>, startup, paused, p: SimParams| {
            let mut g = Gpu {
                short: short.into(),
                bus: bus.into(),
                sub: sub.into(),
                limits,
                applied: profiles[0].tuning.clone(),
                staged: profiles[0].tuning.clone(),
                profiles,
                profile: 0,
                startup,
                now: Sample { temp: 40., util: 5., power: 30., fan: 30., gclk: 300., mclk: 405., fan_rpm: None },
                history: History::new(),
                cooling_paused: paused,
                real: false,
                sim: Some(p),
            };
            for t in 0..HISTORY_SECS as u64 {
                step(&mut g, t);
            }
            g
        };
        vec![
            mk(
                "RTX 4080 SUPER",
                "0000:01:00.0",
                "16 GB GDDR6X · PCIe 4.0 ×16",
                Limits { power_min: 125, power_max: 350, max_clock: 2610., max_mem_clock: 11502. },
                profiles([250, 320, 350], (150, 1000)),
                1,
                false,
                SimParams { game: true, ambient: 30., thermal: 0.12, mem_clock: 11502., seed: 0x9e3779b97f4a7c15 },
            ),
            mk(
                "RTX A2000",
                "0000:05:00.0",
                "12 GB GDDR6 · PCIe 4.0 ×16",
                Limits { power_min: 50, power_max: 70, max_clock: 1200., max_mem_clock: 7001. },
                profiles([55, 65, 70], (0, 0)),
                0,
                true,
                SimParams { game: false, ambient: 32., thermal: 0.42, mem_clock: 7001., seed: 0xdeadbeefcafef00d },
            ),
        ]
    }

    pub fn step(g: &mut Gpu, t: u64) {
        let p = g.sim.as_mut().unwrap();
        let tf = t as f32;
        let target = if p.game {
            if t % 240 < 25 { 18. + noise(&mut p.seed, 4.) } else { 68. + 18. * (tf / 37.).sin() + 8. * (tf / 9.).sin() + noise(&mut p.seed, 5.) }
        } else {
            18. + 14. * (tf / 55.).sin().max(0.) + noise(&mut p.seed, 3.)
        };
        let s = &mut g.now;
        s.util = (s.util + (target - s.util) * 0.35).clamp(0., 100.);
        let pl = g.applied.power_limit as f32;
        let idle = g.limits.power_min as f32 * 0.18;
        let pmax = g.limits.power_max as f32;
        s.power = (idle + (pmax * 0.86 - idle) * s.util / 100. + noise(&mut p.seed, pmax * 0.012)).clamp(0., pl);
        let mut fan_t = match g.applied.fan_mode {
            FanMode::Curve => curve_at(&g.applied.curve, s.temp),
            FanMode::Fixed => g.applied.fan_fixed as f32,
            FanMode::Auto => ((s.temp - 35.) * 1.4 + 30.).clamp(30., 100.),
        };
        if g.cooling_paused {
            fan_t = s.fan;
        }
        s.fan = (s.fan + (fan_t - s.fan) * 0.15).clamp(0., 100.);
        s.fan_rpm = Some((s.fan * 31. + 180.).round() as u32);
        let temp_t = p.ambient + s.power * p.thermal * (1.25 - s.fan / 180.);
        s.temp += (temp_t - s.temp) * 0.06;
        let at_limit = s.power >= pl - 1.5;
        s.gclk = if s.util > 12. {
            g.limits.max_clock + g.applied.g_offset as f32 - if at_limit { 120. + noise(&mut p.seed, 30.) } else { noise(&mut p.seed, 15.) }
        } else {
            210. + noise(&mut p.seed, 10.)
        };
        s.mclk = if s.util > 8. { p.mem_clock + g.applied.m_offset as f32 } else { 405. };
        let s = *s;
        g.history.push(&s);
    }
}

mod nvml {
    use super::*;
    use nvml_wrapper::Nvml;
    use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};

    /// Start a 1 Hz sampling thread over all NVIDIA GPUs. `None` if NVML is unavailable.
    pub fn start() -> Option<(Vec<Gpu>, mpsc::Receiver<Vec<Sample>>)> {
        let nvml = Nvml::init().ok()?;
        let n = nvml.device_count().ok()?;
        if n == 0 {
            return None;
        }
        let mut gpus = Vec::new();
        for i in 0..n {
            let d = nvml.device_by_index(i).ok()?;
            let name = d.name().unwrap_or_else(|_| "NVIDIA GPU".into());
            let short = name.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ").to_string();
            let bus = d.pci_info().map(|p| p.bus_id.to_lowercase()).unwrap_or_default();
            // NVML reports an 8-digit PCI domain ("00000000:01:00.0"); show the usual 4
            let bus = if bus.len() == 16 { bus[4..].to_string() } else { bus };
            let mem_gb = d.memory_info().map(|m| (m.total as f64 / 1e9).round() as u64).unwrap_or(0);
            let pcie = match (d.current_pcie_link_gen(), d.current_pcie_link_width()) {
                (Ok(g), Ok(w)) => format!("PCIe {g}.0 ×{w}"),
                _ => "PCIe".into(),
            };
            let (pmin, pmax) = d.power_management_limit_constraints().map(|c| (c.min_limit / 1000, c.max_limit / 1000)).unwrap_or((100, 450));
            let plimit = d.power_management_limit().map(|w| w / 1000).unwrap_or(pmax);
            let max_clock = d.max_clock_info(Clock::Graphics).map(|c| c as f32).unwrap_or(3000.);
            let max_mem = d.max_clock_info(Clock::Memory).map(|c| c as f32).unwrap_or(15000.);
            let step = |x: u32| (x / 5) * 5;
            let mut profs = profiles([step(pmin + (plimit - pmin) * 2 / 3), plimit, pmax], (150, 1000));
            for p in &mut profs {
                p.tuning.power_limit = p.tuning.power_limit.clamp(pmin, pmax);
            }
            profs[1].tuning.power_limit = plimit;
            let mut applied = profs[1].tuning.clone();
            applied.power_limit = plimit;
            gpus.push(Gpu {
                short,
                bus,
                sub: format!("{mem_gb} GB · {pcie}"),
                limits: Limits { power_min: pmin, power_max: pmax, max_clock, max_mem_clock: max_mem },
                staged: applied.clone(),
                applied,
                profiles: profs,
                profile: 1,
                startup: 1,
                now: Sample::default(),
                history: History::new(),
                cooling_paused: false,
                real: true,
                sim: None,
            });
        }
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("nvml-sampler".into())
            .spawn(move || {
                loop {
                    let mut out = Vec::new();
                    for i in 0..n {
                        let Ok(d) = nvml.device_by_index(i) else { continue };
                        out.push(Sample {
                            temp: d.temperature(TemperatureSensor::Gpu).unwrap_or(0) as f32,
                            util: d.utilization_rates().map(|u| u.gpu as f32).unwrap_or(0.),
                            power: d.power_usage().map(|p| p as f32 / 1000.).unwrap_or(0.),
                            gclk: d.clock_info(Clock::Graphics).unwrap_or(0) as f32,
                            mclk: d.clock_info(Clock::Memory).unwrap_or(0) as f32,
                            fan: d.fan_speed(0).unwrap_or(0) as f32,
                            fan_rpm: d.fan_speed_rpm(0).ok(),
                        });
                    }
                    if tx.send(out).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            })
            .ok()?;
        Some((gpus, rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_counts_and_discards() {
        let mut app = App::new(true);
        assert_eq!(app.gpu().pending_count(), 0);
        app.step(Step::PowerLimit, -5);
        app.drag_curve_point(2, 0.5, 0.2);
        assert_eq!(app.gpu().pending_count(), 2);
        assert!(app.gpu().curve_pending());
        app.discard();
        assert_eq!(app.gpu().pending_count(), 0);
    }

    #[test]
    fn power_drag_snaps_and_clamps() {
        let mut app = App::new(true);
        app.drag_power_limit(0.0);
        assert_eq!(app.gpu().staged.power_limit, 350);
        app.drag_power_limit(1.0);
        assert_eq!(app.gpu().staged.power_limit, 125);
        app.drag_power_limit(0.443);
        assert_eq!(app.gpu().staged.power_limit % 5, 0);
    }

    #[test]
    fn curve_points_keep_order() {
        let mut app = App::new(true);
        app.drag_curve_point(1, 1.0, 0.0);
        let c = &app.gpu().staged.curve;
        assert!(c[1].0 <= c[2].0 - 2.0);
        assert_eq!(c[1].1, 100.0);
    }

    #[test]
    fn history_downsamples() {
        let app = App::new(true);
        let d = app.gpu().history.sample(Metric::Power, 3600, 600);
        assert_eq!(d.len(), 600);
        let (mx, avg, mn) = app.gpu().history.stats(Metric::Power, 300).unwrap();
        assert!(mn <= avg && avg <= mx);
    }

    #[test]
    fn commands_filter() {
        let app = App::new(true);
        assert!(app.commands("quiet").iter().any(|c| c.label == "Apply profile QUIET"));
        assert!(app.commands("zzz").is_empty());
    }
}
