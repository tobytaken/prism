//! PROTOTYPE, throwaway. Toolkit spike (#102): C · Tracks GPU workspace in Slint 1.18.
//!
//! Run: `cargo run --profile spike -p spike-slint [-- --sim]`

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use spike_core::theme::{self, Tokens as CoreTokens};
use spike_core::{self as core, FanMode, Metric, RANGES, Step, curve_to_norm, signed_mhz, y_frac};

slint::include_modules!();

const METRICS: [Metric; 5] = [Metric::Temp, Metric::Util, Metric::Power, Metric::Gclk, Metric::Fan];

fn col(hex: u32) -> slint::Color {
    slint::Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn tokens(t: &CoreTokens) -> Tokens {
    Tokens {
        bg: col(t.bg),
        panel: col(t.panel),
        panel2: col(t.panel2),
        border: col(t.border),
        hover: col(t.hover),
        text: col(t.text),
        muted: col(t.muted),
        accent: col(t.accent),
        danger: col(t.danger),
        ok: col(t.ok),
        grid: col(t.grid),
        line: col(t.line),
        chrome: col(t.chrome),
        chrome_text: col(t.chrome_text),
        chrome_muted: col(t.chrome_muted),
        chrome_border: col(t.chrome_border),
        chrome_hover: col(t.chrome_hover),
        chrome_hi: col(t.chrome_hi),
        on_accent: col(t.on_accent),
    }
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

/// Chart x/y mapping shared by the line, area and hover callbacks:
/// samples are right-aligned in the selected range.
fn points(app: &core::App, m: Metric, w: f32, h: f32) -> Vec<(f32, f32)> {
    let g = app.gpu();
    let data = g.history.sample(m, app.range, 800);
    let slots = app.range.min(800).max(data.len()) as f32;
    let off = slots - data.len() as f32;
    let scale = g.scale(m);
    data.iter().enumerate().map(|(i, v)| (w * (off + i as f32) / (slots - 1.), 2. + (h - 4.) * y_frac(*v, scale))).collect()
}

fn path(pts: &[(f32, f32)]) -> String {
    let mut s = String::with_capacity(pts.len() * 14);
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(s, "{}{x:.1} {y:.1} ", if i == 0 { "M" } else { "L" });
    }
    s
}

fn dashes(from: (f32, f32), to: (f32, f32), on: f32, off: f32) -> String {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    let mut s = String::new();
    let mut d = 0.;
    while d < len {
        let e = (d + on).min(len);
        let _ = write!(s, "M{:.1} {:.1} L{:.1} {:.1} ", from.0 + dx * d / len, from.1 + dy * d / len, from.0 + dx * e / len, from.1 + dy * e / len);
        d += on + off;
    }
    s
}

fn sync(ui: &MainWindow, app: &core::App, rev: &Cell<i32>) {
    let s = ui.global::<S>();
    let th = ui.global::<Theme>();
    th.set_pref(match app.theme_pref {
        core::ThemePref::System => 0,
        core::ThemePref::Light => 1,
        core::ThemePref::Dark => 2,
    });
    let g = app.gpu();
    let n = &g.now;
    s.set_gpus(model(
        app.gpus
            .iter()
            .enumerate()
            .map(|(i, g)| GpuRow {
                idx: format!("GPU {i}").into(),
                name: g.short.clone().into(),
                temp: format!("{:.0}", g.now.temp).into(),
                util: format!("{:.0}%", g.now.util).into(),
                power: format!("{:.0} W", g.now.power).into(),
                fan: format!("{:.0}%", g.now.fan).into(),
                paused: g.cooling_paused,
            })
            .collect(),
    ));
    s.set_sel(app.sel as i32);
    s.set_name(g.short.clone().into());
    s.set_sub(format!("{} · {}", g.bus, g.sub).into());
    s.set_profiles(model(g.profiles.iter().enumerate().map(|(i, p)| Prof { name: p.name.clone().into(), startup: i == g.startup }).collect()));
    s.set_profile(g.profile as i32);
    s.set_modified(g.modified_from_profile());
    s.set_range(RANGES.iter().position(|(r, _)| *r == app.range).unwrap_or(1) as i32);
    let pc = g.pending_count();
    s.set_pending(pc as i32);
    s.set_strip_text(
        if pc == 0 {
            "NO UNAPPLIED CHANGES · EDITS ARE STAGED HERE UNTIL YOU APPLY THEM".into()
        } else {
            format!("{pc} {} · AUTOMATIC CHANGES ON HOLD", if pc == 1 { "UNAPPLIED CHANGE" } else { "UNAPPLIED CHANGES" })
        }
        .into(),
    );
    s.set_temp(format!("{:.0}", n.temp).into());
    s.set_util(format!("{:.0}", n.util).into());
    s.set_power(format!("{:.0}", n.power).into());
    s.set_gclk(format!("{:.0}", n.gclk).into());
    s.set_fan(format!("{:.0}", n.fan).into());
    s.set_rpm(n.fan_rpm.map(|r| format!("{r} rpm")).unwrap_or("— rpm".into()).into());
    s.set_limit(format!("{} W", g.staged.power_limit).into());
    s.set_limit_pending(g.power_pending().map(|p| format!("→ {p} W")).unwrap_or_default().into());
    s.set_limit_label(format!("limit {} W", g.applied.power_limit).into());
    s.set_pending_label(g.power_pending().map(|p| format!("pending {p} W")).unwrap_or_default().into());
    let pscale = g.scale(Metric::Power);
    s.set_limit_frac(y_frac(g.applied.power_limit as f32, pscale));
    s.set_pending_frac(g.power_pending().map(|p| y_frac(p as f32, pscale)).unwrap_or(-1.));
    s.set_goff(signed_mhz(g.staged.g_offset).into());
    s.set_goff_pending(if g.staged.g_offset != g.applied.g_offset { format!("→ {}", signed_mhz(g.staged.g_offset)) } else { String::new() }.into());
    s.set_moff(signed_mhz(g.staged.m_offset).into());
    s.set_moff_pending(if g.staged.m_offset != g.applied.m_offset { format!("→ {}", signed_mhz(g.staged.m_offset)) } else { String::new() }.into());
    s.set_fan_mode(FanMode::ALL.iter().position(|m| *m == g.staged.fan_mode).unwrap_or(0) as i32);
    s.set_paused(g.cooling_paused);
    s.set_curve_open(app.curve_open);
    s.set_curve(model(g.staged.curve.iter().map(|p| curve_to_norm(*p)).map(|(x, y)| Pt { x, y }).collect()));
    s.set_curve_pending(g.curve_pending());
    let (nx, ny) = curve_to_norm((n.temp.clamp(20., 100.), n.fan));
    s.set_now(Pt { x: nx, y: ny });
    s.set_now_label(format!("now {:.0}°C · fan {:.0}%", n.temp, n.fan).into());
    s.set_stats(model(
        METRICS
            .iter()
            .map(|m| match g.history.stats(*m, app.range) {
                Some((mx, av, mn)) => Stat3 { max: format!("{mx:.0}").into(), avg: format!("{av:.0}").into(), min: format!("{mn:.0}").into() },
                None => Stat3 { max: "".into(), avg: "".into(), min: "".into() },
            })
            .collect(),
    ));
    s.set_daemon(app.status_daemon().into());
    s.set_problems(if app.problems() > 0 { format!("▲ {} problem", app.problems()) } else { String::new() }.into());
    s.set_theme_label(app.theme_label().into());
    rev.set(rev.get() + 1);
    s.set_rev(rev.get());
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let app = Rc::new(RefCell::new(core::App::from_args()));
    let rev = Rc::new(Cell::new(0));
    let palette_q = Rc::new(RefCell::new(String::new()));
    ui.global::<Theme>().set_light(tokens(&theme::LIGHT));
    ui.global::<Theme>().set_dark(tokens(&theme::DARK));
    sync(&ui, &app.borrow(), &rev);

    let s = ui.global::<S>();
    // -- geometry, asked for by the view --
    {
        let a = app.clone();
        s.on_series(move |m, w, h, _| path(&points(&a.borrow(), METRICS[m as usize], w, h)).into());
        let a = app.clone();
        s.on_area(move |m, w, h, _| {
            let p = points(&a.borrow(), METRICS[m as usize], w, h);
            if p.len() < 2 {
                return SharedString::new();
            }
            format!("M{:.1} {h} {}L{:.1} {h} Z", p[0].0, path(&p).replacen('M', "L", 1), p[p.len() - 1].0).into()
        });
        let a = app.clone();
        s.on_value_at(move |m, x, _| {
            let a = a.borrow();
            let m = METRICS[m as usize];
            let d = a.gpu().history.sample(m, a.range, 800);
            let slots = a.range.min(800).max(d.len());
            let slot = (x * slots as f32) as isize - (slots - d.len()) as isize;
            let unit = match m {
                Metric::Temp => "°C",
                Metric::Util | Metric::Fan => "%",
                Metric::Power => "W",
                _ => "MHz",
            };
            if slot >= 0 && (slot as usize) < d.len() { format!("{:.0} {unit}", d[slot as usize]).into() } else { SharedString::new() }
        });
        s.on_dash_h(|w, y| dashes((0., y), (w, y), 4., 3.).into());
        s.on_dash_v(|x, h| dashes((x + 0.5, 0.), (x + 0.5, h), 3., 3.).into());
        let a = app.clone();
        s.on_curve_line(move |old, w, h, _| {
            let a = a.borrow();
            let g = a.gpu();
            let c = if old { &g.applied.curve } else { &g.staged.curve };
            let mut pts = vec![(0., curve_to_norm(c[0]).1 * h)];
            pts.extend(c.iter().map(|p| curve_to_norm(*p)).map(|(x, y)| (x * w, y * h)));
            pts.push((w, curve_to_norm(c[c.len() - 1]).1 * h));
            if old {
                pts.windows(2).map(|s| dashes(s[0], s[1], 4., 3.)).collect::<String>().into()
            } else {
                path(&pts).into()
            }
        });
        let a = app.clone();
        s.on_curve_fill(move |w, h, _| {
            let a = a.borrow();
            let c = &a.gpu().staged.curve;
            let mut pts = vec![(0., curve_to_norm(c[0]).1 * h)];
            pts.extend(c.iter().map(|p| curve_to_norm(*p)).map(|(x, y)| (x * w, y * h)));
            pts.push((w, curve_to_norm(c[c.len() - 1]).1 * h));
            format!("{}L{w} {h} L0 {h} Z", path(&pts)).into()
        });
        let a = app.clone();
        s.on_curve_label(move |i, _| {
            let a = a.borrow();
            a.gpu().staged.curve.get(i as usize).map(|p| format!("{:.0}° → {:.0}%", p.0, p.1)).unwrap_or_default().into()
        });
    }
    // -- intents --
    macro_rules! intent {
        ($on:ident, |$a:ident $(, $arg:ident)*| $body:expr) => {{
            let (app, rev, w) = (app.clone(), rev.clone(), ui.as_weak());
            s.$on(move |$($arg),*| {
                {
                    let mut $a = app.borrow_mut();
                    $body;
                }
                if let Some(ui) = w.upgrade() {
                    sync(&ui, &app.borrow(), &rev);
                }
            });
        }};
    }
    intent!(on_select_gpu, |a, i| a.select(i as usize));
    intent!(on_apply_profile, |a, i| a.apply_profile(i as usize));
    intent!(on_set_range, |a, i| a.range = RANGES[i as usize].0);
    intent!(on_step, |a, what, dir| a.step([Step::PowerLimit, Step::GOffset, Step::MOffset][what as usize], dir));
    intent!(on_drag_power, |a, f| a.drag_power_limit(f));
    intent!(on_set_fan_mode, |a, i| a.set_fan_mode(FanMode::ALL[i as usize]));
    intent!(on_toggle_curve, |a| a.curve_open = !a.curve_open);
    intent!(on_curve_drag, |a, i, x, y| a.drag_curve_point(i as usize, x, y));
    intent!(on_apply, |a| a.apply());
    intent!(on_discard, |a| a.discard());
    intent!(on_cycle_theme, |a| a.cycle_theme());
    {
        let a = app.clone();
        let w = ui.as_weak();
        s.on_curve_hit(move |x, y, aspect| {
            let h = w.upgrade().map(|u| u.get_curve_h()).unwrap_or(200.);
            a.borrow().curve_hit(x, y, aspect, 12. / h).map(|i| i as i32).unwrap_or(-1)
        });
    }
    // -- command palette --
    let set_commands = {
        let (a, q) = (app.clone(), palette_q.clone());
        move |ui: &MainWindow| {
            let cmds: Vec<SharedString> = a.borrow().commands(&q.borrow()).into_iter().take(12).map(|c| c.label.into()).collect();
            ui.global::<S>().set_commands(model(cmds));
        }
    };
    {
        let (w, q, sc) = (ui.as_weak(), palette_q.clone(), set_commands.clone());
        s.on_open_palette(move || {
            let ui = w.unwrap();
            q.borrow_mut().clear();
            sc(&ui);
            ui.global::<S>().set_palette_sel(0);
            ui.global::<S>().set_palette_open(true);
        });
        let (w, q, sc) = (ui.as_weak(), palette_q.clone(), set_commands.clone());
        s.on_palette_query(move |text| {
            *q.borrow_mut() = text.to_string();
            sc(&w.unwrap());
        });
        let (w, a, q, r) = (ui.as_weak(), app.clone(), palette_q.clone(), rev.clone());
        s.on_palette_run(move |i| {
            let ui = w.unwrap();
            let cmd = a.borrow().commands(&q.borrow()).into_iter().nth(i as usize);
            if let Some(c) = cmd {
                a.borrow_mut().run(c.action);
            }
            ui.global::<S>().set_palette_open(false);
            ui.invoke_focus_keys();
            sync(&ui, &a.borrow(), &r);
        });
        let w = ui.as_weak();
        s.on_close_palette(move || {
            let ui = w.unwrap();
            ui.global::<S>().set_palette_open(false);
            ui.invoke_focus_keys();
        });
    }
    // -- window controls --
    {
        let w = ui.as_weak();
        s.on_win_minimize(move || w.unwrap().window().set_minimized(true));
        let w = ui.as_weak();
        s.on_win_maximize(move || {
            let ui = w.unwrap();
            let m = ui.window().is_maximized();
            ui.window().set_maximized(!m);
        });
        let w = ui.as_weak();
        s.on_win_close(move || {
            let _ = w.unwrap().hide();
        });
    }
    // -- 1 Hz telemetry --
    let timer = slint::Timer::default();
    {
        let (a, r, w) = (app.clone(), rev.clone(), ui.as_weak());
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            a.borrow_mut().tick();
            if let Some(ui) = w.upgrade() {
                sync(&ui, &a.borrow(), &r);
            }
        });
    }
    // -- frame counter for idle measurements --
    let frames = Rc::new(Cell::new(0u64));
    if std::env::var_os("SPIKE_STATS").is_some() {
        let f = frames.clone();
        let _ = ui.window().set_rendering_notifier(move |state, _| {
            if let slint::RenderingState::AfterRendering = state {
                f.set(f.get() + 1);
            }
        });
    }
    let stats_timer = slint::Timer::default();
    if std::env::var_os("SPIKE_STATS").is_some() {
        let (f, start) = (frames.clone(), Instant::now());
        stats_timer.start(slint::TimerMode::Repeated, Duration::from_secs(10), move || {
            eprintln!("[spike] t={:>4}s frames={}", start.elapsed().as_secs(), f.get());
        });
    }
    // -- system color scheme, as Slint sees it (for the log) --
    {
        let w = ui.as_weak();
        let last = Cell::new(None);
        let t = slint::Timer::default();
        t.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
            if let Some(ui) = w.upgrade() {
                let d = ui.global::<Theme>().get_is_dark();
                if last.get() != Some(d) {
                    eprintln!("[spike] effective dark={d}");
                    last.set(Some(d));
                }
            }
        });
        std::mem::forget(t);
    }
    if std::env::var_os("SPIKE_SELFTEST").is_some() {
        selftest(&ui, app.clone());
    }
    ui.run()
}

/// Scripted pointer input through Slint's real dispatch path while the 1 Hz
/// updates keep redrawing: drag the power-limit line, then a fan-curve point.
fn selftest(ui: &MainWindow, app: Rc<RefCell<core::App>>) {
    use slint::platform::{PointerEventButton, WindowEvent};
    use slint::LogicalPosition;
    let w = ui.as_weak();
    let step = Rc::new(Cell::new(0));
    let t = slint::Timer::default();
    let start = Rc::new(RefCell::new(None::<(f32, f32, f32, f32)>));
    let curve0 = app.borrow().gpu().staged.curve[2];
    let delay: i32 = std::env::var("SPIKE_SELFTEST_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let loops = Cell::new(std::env::var("SPIKE_SELFTEST_LOOPS").ok().and_then(|v| v.parse::<i32>().ok()).unwrap_or(1));
    t.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
        let Some(ui) = w.upgrade() else { return };
        let win = ui.window();
        // steps 0..60 are a 3 s settle; stretch it to SPIKE_SELFTEST_DELAY seconds
        let raw = step.get();
        step.set(raw + 1);
        let i = if raw < delay * 20 { raw * 60 / (delay * 20).max(1) } else { raw - delay * 20 + 60 };
        let i = if raw < delay * 20 && i >= 60 { 59 } else { i };
        let send = |e| win.dispatch_event(e);
        let left = PointerEventButton::Left;
        match i {
            0..60 => {} // 3 s settle
            60 => {
                let (x, y, w_, h) = (ui.get_power_x(), ui.get_power_y(), ui.get_power_w(), ui.get_power_h());
                *start.borrow_mut() = Some((x, y, w_, h));
                let p = LogicalPosition::new(x + w_ * 0.6, y + h * 0.45);
                send(WindowEvent::PointerMoved { position: p });
                send(WindowEvent::PointerPressed { position: p, button: left });
            }
            61..91 => {
                let (x, y, w_, h) = start.borrow().unwrap();
                let p = LogicalPosition::new(x + w_ * 0.6, y + h * (0.45 - 0.004 * (i - 60) as f32));
                send(WindowEvent::PointerMoved { position: p });
            }
            91 => {
                let (x, y, w_, h) = start.borrow().unwrap();
                send(WindowEvent::PointerReleased { position: LogicalPosition::new(x + w_ * 0.6, y + h * (0.45 - 0.12)), button: left });
                let a = app.borrow();
                eprintln!("[selftest] power: applied={} staged={} pending={}", a.gpu().applied.power_limit, a.gpu().staged.power_limit, a.gpu().pending_count());
            }
            92 => {
                let (nx, ny) = curve_to_norm(curve0);
                let p = LogicalPosition::new(ui.get_curve_x() + ui.get_curve_w() * nx, ui.get_curve_y() + ui.get_curve_h() * ny);
                *start.borrow_mut() = Some((p.x, p.y, 0., 0.));
                send(WindowEvent::PointerMoved { position: p });
                send(WindowEvent::PointerPressed { position: p, button: left });
            }
            93..123 => {
                let (x, y, ..) = start.borrow().unwrap();
                let k = (i - 92) as f32;
                send(WindowEvent::PointerMoved { position: LogicalPosition::new(x - k, y - 3. * k) });
            }
            123 => {
                let (x, y, ..) = start.borrow().unwrap();
                send(WindowEvent::PointerReleased { position: LogicalPosition::new(x - 30., y - 90.), button: left });
                let a = app.borrow();
                eprintln!("[selftest] curve: before={:?} after={:?} pending={}", curve0, a.gpu().staged.curve[2], a.gpu().pending_count());
                drop(a);
                app.borrow_mut().discard();
                if loops.get() > 1 {
                    loops.set(loops.get() - 1);
                    step.set(delay * 20); // back to i = 60: the power-line press
                }
            }
            124 => {
                let ctrl = slint::platform::Key::Control;
                send(WindowEvent::KeyPressed { text: SharedString::from(ctrl) });
                send(WindowEvent::KeyPressed { text: "k".into() });
                send(WindowEvent::KeyReleased { text: "k".into() });
                send(WindowEvent::KeyReleased { text: SharedString::from(ctrl) });
            }
            130 | 132 | 134 | 136 => {
                let ch = ["p", "e", "r", "f"][((i - 130) / 2) as usize];
                send(WindowEvent::KeyPressed { text: ch.into() });
                send(WindowEvent::KeyReleased { text: ch.into() });
            }
            140 => {
                eprintln!("[selftest] palette open={} commands={}", ui.global::<S>().get_palette_open(), slint::Model::row_count(&ui.global::<S>().get_commands()));
            }
            156 => {
                send(WindowEvent::KeyPressed { text: "\n".into() });
                send(WindowEvent::KeyReleased { text: "\n".into() });
            }
            160 => {
                let a = app.borrow();
                eprintln!("[selftest] palette: profile={} open={}", a.gpu().profiles[a.gpu().profile].name, ui.global::<S>().get_palette_open());
                eprintln!("[selftest] done");
            }
            _ => {}
        }
    });
    std::mem::forget(t);
}
