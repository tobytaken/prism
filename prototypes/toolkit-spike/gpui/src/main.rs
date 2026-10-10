//! PROTOTYPE, throwaway. Toolkit spike (#102): C · Tracks GPU workspace in gpui (gpui-kit 0.7.1).
//!
//! Run: `cargo run --profile spike -p spike-gpui [-- --sim]`

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use spike_core::theme::{FONT_MONO, FONT_UI, Tokens};
use spike_core::{
    self as core, FanMode, Metric, RANGES, Step, curve_to_norm, signed_mhz, y_frac,
};

actions!(prism, [OpenPalette, ApplyEdits, Escape]);

const INSET: f32 = 8.0;

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Power,
    Curve(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum Chart {
    Temp,
    Util,
    Power,
    Clocks,
    Fan,
}

struct Palette {
    input: Entity<InputState>,
    sel: usize,
    _sub: Subscription,
}

struct Prism {
    core: core::App,
    focus: FocusHandle,
    palette: Option<Palette>,
    drag: Option<Drag>,
    hover: Option<(Chart, f32)>,
    curve_hover: Option<usize>,
    chart_bounds: [Rc<Cell<Bounds<Pixels>>>; 5],
    curve_bounds: Rc<Cell<Bounds<Pixels>>>,
    frames: Rc<Cell<u64>>,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex: u32, a: f32) -> Hsla {
    let mut h: Hsla = rgb(hex).into();
    h.a = a;
    h
}

impl Prism {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut core = core::App::from_args();
        core.system_dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
        cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
            this.core.system_dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
            eprintln!("[spike] system appearance changed: dark={}", this.core.system_dark);
            cx.notify();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update(cx, |this, cx| {
                        this.core.tick();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let frames = Rc::new(Cell::new(0));
        if std::env::var_os("SPIKE_STATS").is_some() {
            let f = frames.clone();
            let start = Instant::now();
            cx.spawn(async move |_, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(10)).await;
                    eprintln!("[spike] t={:>4}s frames={}", start.elapsed().as_secs(), f.get());
                }
            })
            .detach();
        }
        if std::env::var_os("SPIKE_SELFTEST").is_some() {
            cx.spawn_in(window, async move |this, cx| selftest(this, cx).await).detach();
        }
        Self {
            core,
            focus: cx.focus_handle(),
            palette: None,
            drag: None,
            hover: None,
            curve_hover: None,
            chart_bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            curve_bounds: Rc::new(Cell::new(Bounds::default())),
            frames,
        }
    }

    fn t(&self) -> &'static Tokens {
        self.core.tokens()
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a command…"));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                if let Some(p) = &mut this.palette {
                    p.sel = 0;
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.run_palette(window, cx),
            _ => {}
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.palette = Some(Palette { input, sel: 0, _sub: sub });
        cx.notify();
    }

    fn palette_query(&self, cx: &App) -> String {
        self.palette.as_ref().map(|p| p.input.read(cx).value().to_string()).unwrap_or_default()
    }

    fn run_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let q = self.palette_query(cx);
        let sel = self.palette.as_ref().map(|p| p.sel).unwrap_or(0);
        if let Some(cmd) = self.core.commands(&q).into_iter().nth(sel) {
            self.core.run(cmd.action);
        }
        self.close_palette(window, cx);
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // ---------- pointer handling for the on-chart controls ----------

    fn power_pointer(&mut self, pos: Point<Pixels>) {
        let b = self.chart_bounds[Chart::Power as usize].get();
        let f = ((pos.y - b.origin.y) / b.size.height).clamp(0.0, 1.0);
        self.core.drag_power_limit(f);
    }

    fn curve_norm(&self, pos: Point<Pixels>) -> (f32, f32, f32) {
        let b = self.curve_bounds.get();
        let nx = (pos.x - b.origin.x) / b.size.width;
        let ny = (pos.y - b.origin.y) / b.size.height;
        (nx, ny, b.size.width / b.size.height)
    }

    fn curve_hit(&self, pos: Point<Pixels>) -> Option<usize> {
        let (nx, ny, aspect) = self.curve_norm(pos);
        let b = self.curve_bounds.get();
        self.core.curve_hit(nx, ny, aspect, px(12.0) / b.size.height)
    }

    // ---------- view pieces ----------

    fn mono(&self) -> Div {
        div().font_family(FONT_MONO)
    }

    fn label(&self, s: impl Into<SharedString>) -> Div {
        self.mono().text_size(px(10.)).text_color(c(self.t().muted)).child(s.into())
    }

    fn titlebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t();
        let btn = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(22.))
                .px(px(8.))
                .flex()
                .items_center()
                .font_family(FONT_MONO)
                .text_size(px(11.))
                .text_color(c(t.chrome_muted))
                .hover(|s| s.text_color(c(t.chrome_hi)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label)
        };
        let wc = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(t.chrome_muted))
                .hover(|s| s.bg(c(t.chrome_hover)).text_color(c(t.chrome_hi)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label)
        };
        let _ = window;
        div()
            .id("titlebar")
            .h(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(12.))
            .pr(px(4.))
            .bg(c(t.chrome))
            .text_color(c(t.chrome_text))
            .on_mouse_down(MouseButton::Left, |e, window, _| {
                if e.click_count == 2 {
                    window.zoom_window();
                } else {
                    window.start_window_move();
                }
            })
            .on_mouse_down(MouseButton::Right, |e, window, _| window.show_window_menu(e.position))
            .child(
                self.mono()
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(11.))
                    .flex()
                    .child("PRI")
                    .child(div().text_color(c(t.accent)).child("S"))
                    .child("M"),
            )
            .child(div().flex_1())
            .child(
                btn("palette", "COMMANDS")
                    .child(
                        div()
                            .ml(px(8.))
                            .px(px(4.))
                            .border_1()
                            .border_color(c(t.chrome_border))
                            .rounded(px(3.))
                            .text_size(px(10.))
                            .child("CTRL K"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx))),
            )
            .child(div().flex_1())
            .child(btn("theme", self.core.theme_label()).on_click(cx.listener(|this, _, _, cx| {
                this.core.cycle_theme();
                cx.notify();
            })))
            .child(btn("compact", "COMPACT"))
            .child(wc("min", "–").on_click(|_, window, _| window.minimize_window()))
            .child(wc("max", "□").on_click(|_, window, _| window.zoom_window()))
            .child(wc("close", "×").on_click(|_, window, _| window.remove_window()))
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t();
        let mut side = div().w(px(196.)).flex_none().flex().flex_col().border_r_1().border_color(c(t.border)).bg(c(t.bg));
        for (i, g) in self.core.gpus.iter().enumerate() {
            let sel = i == self.core.sel;
            let n = &g.now;
            side = side.child(
                div()
                    .id(("gpu", i))
                    .relative()
                    .px(px(12.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(c(t.border))
                    .when(sel, |d| d.bg(c(t.panel2)))
                    .when(!sel, |d| d.hover(|s| s.bg(c(t.hover))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.core.select(i);
                        cx.notify();
                    }))
                    .child(
                        self.mono()
                            .flex()
                            .text_size(px(10.))
                            .text_color(c(t.muted))
                            .child(format!("GPU {i}"))
                            .child(div().flex_1())
                            .when(g.cooling_paused, |d| d.child(div().text_color(c(t.danger)).child("■ PAUSED"))),
                    )
                    .child(div().font_weight(FontWeight::SEMIBOLD).mt(px(2.)).mb(px(4.)).child(g.short.clone()))
                    .child(
                        self.mono()
                            .flex()
                            .items_baseline()
                            .child(div().text_size(px(30.)).line_height(px(30.)).child(format!("{:.0}", n.temp)))
                            .child(div().text_size(px(14.)).text_color(c(t.muted)).child("°C")),
                    )
                    .child(
                        self.mono()
                            .flex()
                            .gap(px(10.))
                            .mt(px(6.))
                            .text_size(px(10.))
                            .text_color(c(t.muted))
                            .child(div().text_color(c(t.text)).child(format!("{:.0}%", n.util)))
                            .child(div().text_color(c(t.text)).child(format!("{:.0} W", n.power)))
                            .child(div().flex().child("fan ").child(div().text_color(c(t.text)).child(format!("{:.0}%", n.fan)))),
                    )
                    .when(sel, |d| d.child(div().absolute().top_0().bottom_0().right(px(-1.)).w(px(3.)).bg(c(t.accent)))),
            );
        }
        side = side.child(div().flex_1());
        let mut navs = div().border_t_1().border_color(c(t.border));
        for (i, l) in ["GAMES", "DRIVERS", "SETTINGS"].into_iter().enumerate() {
            navs = navs.child(
                self.mono()
                    .id(("nav", i))
                    .px(px(12.))
                    .py(px(9.))
                    .text_size(px(11.))
                    .text_color(c(t.muted))
                    .border_b_1()
                    .border_color(c(t.border))
                    .hover(|s| s.bg(c(t.hover)).text_color(c(t.text)))
                    .child(l),
            );
        }
        side.child(navs)
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t();
        let g = self.core.gpu();
        let mut profs = div().flex().border_1().border_color(c(t.text));
        for (i, p) in g.profiles.iter().enumerate() {
            let on = i == g.profile;
            profs = profs.child(
                self.mono()
                    .id(("prof", i))
                    .relative()
                    .px(px(10.))
                    .py(px(5.))
                    .text_size(px(11.))
                    .when(i > 0, |d| d.border_l_1().border_color(c(t.text)))
                    .when(on, |d| d.bg(c(t.text)).text_color(c(t.panel)))
                    .when(!on, |d| d.hover(|s| s.bg(c(t.hover))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.core.apply_profile(i);
                        cx.notify();
                    }))
                    .child(p.name.clone())
                    .when(i == g.startup, |d| {
                        d.child(div().absolute().top(px(-2.)).right(px(2.)).text_size(px(8.)).text_color(c(t.accent)).child("●"))
                    }),
            );
        }
        profs = profs.child(self.mono().px(px(8.)).py(px(5.)).text_size(px(11.)).border_l_1().border_color(c(t.text)).child("⋯"));
        let mut seg = self.mono().flex().text_size(px(11.));
        for (r, l) in RANGES {
            let on = r == self.core.range;
            seg = seg.child(
                div()
                    .id(("range", r))
                    .px(px(6.))
                    .py(px(3.))
                    .text_color(c(if on { t.text } else { t.muted }))
                    .when(on, |d| d.border_b_2().border_color(c(t.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.core.range = r;
                        cx.notify();
                    }))
                    .child(l),
            );
        }
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(10.))
            .border_b_1()
            .border_color(c(t.border))
            .child(
                div()
                    .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(g.short.clone()))
                    .child(self.label(format!("{} · {}", g.bus, g.sub))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(3.))
                    .child(profs)
                    .child(
                        self.mono()
                            .h(px(12.))
                            .text_size(px(10.))
                            .text_color(c(t.accent))
                            .when(g.modified_from_profile(), |d| d.child("· not saved to profile")),
                    ),
            )
            .child(seg)
    }

    fn apply_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t();
        let n = self.core.gpu().pending_count();
        let base = self
            .mono()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.))
            .h(px(30.))
            .pl(px(14.))
            .pr(px(8.))
            .border_b_1()
            .text_size(px(11.));
        if n == 0 {
            return base
                .border_color(c(t.border))
                .text_color(c(t.muted))
                .child("NO UNAPPLIED CHANGES · EDITS ARE STAGED HERE UNTIL YOU APPLY THEM");
        }
        let word = if n == 1 { "UNAPPLIED CHANGE" } else { "UNAPPLIED CHANGES" };
        base.bg(c(t.accent))
            .border_color(c(t.accent))
            .text_color(c(t.on_accent))
            .child(format!("{n} {word} · AUTOMATIC CHANGES ON HOLD"))
            .child(div().flex_1())
            .child(
                div()
                    .id("discard")
                    .h(px(22.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(c(t.on_accent))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.discard();
                        cx.notify();
                    }))
                    .child("Discard"),
            )
            .child(
                div()
                    .id("apply")
                    .h(px(22.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .bg(c(t.on_accent))
                    .text_color(c(t.accent))
                    .font_weight(FontWeight::BOLD)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.apply();
                        cx.notify();
                    }))
                    .child("Apply"),
            )
    }

    fn stepper(&self, id: &'static str, value: String, what: Step, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t();
        let b = |id: (&'static str, usize), s: &'static str, dir: i32, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(c(t.text)).text_color(c(t.panel)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.core.step(what, dir);
                    cx.notify();
                }))
                .child(s)
        };
        self.mono()
            .flex()
            .items_center()
            .border_1()
            .border_color(c(t.text))
            .text_size(px(11.))
            .child(b((id, 0), "−", -1, cx))
            .child(div().min_w(px(64.)).flex().justify_center().child(value))
            .child(b((id, 1), "+", 1, cx))
    }

    fn ctlrow(&self, label: &'static str, ctl: impl IntoElement, pending: Option<String>) -> impl IntoElement {
        let t = self.t();
        self.mono()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(11.))
            .child(div().w(px(44.)).text_color(c(t.muted)).child(label))
            .child(ctl)
            .when_some(pending, |d, p| d.child(div().text_color(c(t.accent)).child(p)))
    }

    fn track(&self, h: Option<f32>, left: impl IntoElement, chart: impl IntoElement, stats: Option<Metric>) -> impl IntoElement {
        let t = self.t();
        let g = self.core.gpu();
        let mut right = self.mono().w(px(96.)).flex_none().px(px(10.)).py(px(8.)).border_l_1().border_color(c(t.border)).text_size(px(10.)).text_color(c(t.muted)).flex().flex_col().gap(px(2.));
        if let Some((mx, av, mn)) = stats.and_then(|m| g.history.stats(m, self.core.range)) {
            for (l, v) in [("max", mx), ("avg", av), ("min", mn)] {
                right = right.child(div().flex().gap(px(6.)).child(l).child(div().text_color(c(t.text)).child(format!("{v:.0}"))));
            }
        }
        div()
            .flex()
            .border_b_1()
            .border_color(c(t.border))
            .map(|d| match h {
                Some(h) => d.h(px(h)).flex_none(),
                None => d.flex_1().min_h(px(150.)),
            })
            .child(div().w(px(224.)).flex_none().px(px(14.)).py(px(8.)).flex().flex_col().gap(px(4.)).border_r_1().border_color(c(t.border)).child(left))
            .child(div().flex_1().min_w_0().px(px(8.)).py(px(6.)).flex().gap(px(8.)).child(chart))
            .child(right)
    }

    fn big(&self, v: String, unit: &'static str) -> impl IntoElement {
        self.mono()
            .flex()
            .items_baseline()
            .child(div().text_size(px(22.)).line_height(px(23.)).child(v))
            .child(div().text_size(px(12.)).text_color(c(self.t().muted)).child(unit))
    }

    fn tk(&self, s: &'static str) -> Div {
        self.label(s)
    }

    /// A live line/area chart with hover crosshair. `limit` draws the power-limit line(s).
    fn chart(&self, id: Chart, m: Metric, area: bool, caption: Option<&'static str>, limit: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *self.t();
        let g = self.core.gpu();
        let scale = g.scale(m);
        let data = g.history.sample(m, self.core.range, 800);
        let full = self.core.range.min(core::HISTORY_SECS);
        let applied = g.applied.power_limit as f32;
        let pending = g.power_pending().map(|p| p as f32);
        let hover = self.hover.filter(|(c, _)| *c == id).map(|(_, x)| x);
        let bounds_out = self.chart_bounds[id as usize].clone();
        let n = data.len();
        let entity = cx.entity();
        let dragging = limit && self.drag == Some(Drag::Power);
        let hover_val = hover.and_then(|x| {
            // data is right-aligned in the range window
            let slot = (x * (full.min(800).max(n)) as f32) as isize - (full.min(800).max(n) - n) as isize;
            (slot >= 0 && (slot as usize) < n).then(|| data[slot as usize])
        });
        let unit = match m {
            Metric::Temp => "°C",
            Metric::Util | Metric::Fan => "%",
            Metric::Power => "W",
            _ => "MHz",
        };
        let canvas_el = canvas(
            move |b, _, _| bounds_out.set(b),
            move |b, _, window, _| {
                let (x0, y0, w, h) = (b.origin.x, b.origin.y, b.size.width, b.size.height);
                // grid: 3 rows, 6 cols, 1px hairlines
                for i in 1..=3 {
                    let y = (y0 + h * (i as f32 / 4.)).round();
                    window.paint_quad(fill(Bounds::new(point(x0, y), size(w, px(1.))), c(t.grid)));
                }
                for i in 1..6 {
                    let x = (x0 + w * (i as f32 / 6.)).round();
                    window.paint_quad(fill(Bounds::new(point(x, y0), size(px(1.), h)), c(t.grid)));
                }
                if data.len() >= 2 {
                    let slots = full.min(800).max(data.len()) as f32;
                    let off = slots - data.len() as f32;
                    let px_at = |i: usize, v: f32| point(x0 + w * ((off + i as f32) / (slots - 1.)), y0 + px(2.) + (h - px(4.)) * y_frac(v, scale));
                    if area {
                        let mut p = PathBuilder::fill();
                        p.move_to(point(px_at(0, data[0]).x, y0 + h));
                        for (i, v) in data.iter().enumerate() {
                            p.line_to(px_at(i, *v));
                        }
                        p.line_to(point(px_at(data.len() - 1, 0.).x, y0 + h));
                        p.close();
                        if let Ok(p) = p.build() {
                            window.paint_path(p, linear_gradient(180., linear_color_stop(ca(t.line, 0.28), 0.), linear_color_stop(ca(t.line, 0.), 1.)));
                        }
                    }
                    let mut p = PathBuilder::stroke(px(1.4));
                    for (i, v) in data.iter().enumerate() {
                        if i == 0 { p.move_to(px_at(i, *v)) } else { p.line_to(px_at(i, *v)) }
                    }
                    if let Ok(p) = p.build() {
                        window.paint_path(p, c(t.line));
                    }
                }
                if limit {
                    let ly = |v: f32| (y0 + px(2.) + (h - px(4.)) * y_frac(v, scale)).round();
                    let y = ly(applied);
                    let mut p = PathBuilder::stroke(px(1.5));
                    if pending.is_some() {
                        p = p.dash_array(&[px(4.), px(3.)]);
                    }
                    p.move_to(point(x0, y));
                    p.line_to(point(x0 + w, y));
                    if let Ok(p) = p.build() {
                        window.paint_path(p, c(t.accent));
                    }
                    let hy = if let Some(pv) = pending {
                        let py = ly(pv);
                        window.paint_quad(fill(Bounds::new(point(x0, py - px(1.)), size(w, px(2.5))), c(t.accent)));
                        py
                    } else {
                        y
                    };
                    window.paint_quad(fill(Bounds::new(point(x0 + w - px(10.), hy - px(4.)), size(px(8.), px(8.))), c(t.accent)));
                }
                if dragging {
                    let e1 = entity.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            e1.update(cx, |this, cx| {
                                // listeners live until the next frame: ignore them once the drag ended
                                if this.drag == Some(Drag::Power) {
                                    this.power_pointer(e.position);
                                    cx.notify();
                                }
                            });
                        }
                    });
                    let e2 = entity.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            e2.update(cx, |this, cx| {
                                this.drag = None;
                                cx.notify();
                            });
                        }
                    });
                }
                if let Some(x) = hover {
                    let hx = (x0 + w * x).round();
                    let mut p = PathBuilder::stroke(px(1.)).dash_array(&[px(3.), px(3.)]);
                    p.move_to(point(hx, y0));
                    p.line_to(point(hx, y0 + h));
                    if let Ok(p) = p.build() {
                        window.paint_path(p, ca(t.text, 0.45));
                    }
                }
            },
        )
        .size_full();

        let mut el = div()
            .id(match id {
                Chart::Temp => "c-temp",
                Chart::Util => "c-util",
                Chart::Power => "c-power",
                Chart::Clocks => "c-clocks",
                Chart::Fan => "c-fan",
            })
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(canvas_el)
            .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                if !*h && this.hover.map(|(c, _)| c) == Some(id) {
                    this.hover = None;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                if this.drag.is_some() {
                    return;
                }
                let b = this.chart_bounds[id as usize].get();
                this.hover = Some((id, ((e.position.x - b.origin.x) / b.size.width).clamp(0., 1.)));
                cx.notify();
            }));
        if let Some(cap) = caption {
            el = el.child(self.label(cap).absolute().left(px(6.)).top(px(2.)).text_size(px(9.)));
        }
        if limit {
            let scale_y = |v: f32| relative(0.0f32.max(y_frac(v, scale)));
            let applied_lbl = format!("limit {} W", g.applied.power_limit);
            el = el
                .cursor(CursorStyle::ResizeUpDown)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, e: &MouseDownEvent, _, cx| {
                        this.drag = Some(Drag::Power);
                        this.hover = None;
                        this.power_pointer(e.position);
                        cx.notify();
                    }),
                )
                .child(
                    self.mono()
                        .absolute()
                        .right(px(4.))
                        .top(scale_y(applied))
                        .mt(px(-15.))
                        .text_size(px(10.))
                        .text_color(c(t.accent))
                        .child(applied_lbl),
                )
                .when_some(pending, |d, pv| {
                    d.child(
                        self.mono()
                            .absolute()
                            .left(px(6.))
                            .top(scale_y(pv))
                            .mt(px(-15.))
                            .text_size(px(10.))
                            .text_color(c(t.accent))
                            .child(format!("pending {pv:.0} W")),
                    )
                });
        }
        if let (Some(x), Some(v)) = (hover, hover_val) {
            el = el.child(
                self.mono()
                    .absolute()
                    .top(px(2.))
                    .left(relative(x))
                    .ml(px(6.))
                    .text_size(px(10.))
                    .text_color(c(t.text))
                    .child(format!("{v:.0} {unit}")),
            );
        }
        el
    }

    fn curve_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *self.t();
        let g = self.core.gpu();
        let pts: Vec<(f32, f32)> = g.staged.curve.iter().map(|p| curve_to_norm(*p)).collect();
        let old: Option<Vec<(f32, f32)>> = g.curve_pending().then(|| g.applied.curve.iter().map(|p| curve_to_norm(*p)).collect());
        let pending = g.curve_pending();
        let now = curve_to_norm((g.now.temp.clamp(20., 100.), g.now.fan));
        let drag = match self.drag {
            Some(Drag::Curve(i)) => Some(i),
            _ => None,
        };
        let hl = drag.or(self.curve_hover);
        let bounds_out = self.curve_bounds.clone();
        let entity = cx.entity();
        let dragging = drag.is_some();
        let plot = canvas(
            move |b, _, _| bounds_out.set(b),
            move |b, _, window, _| {
                let (x0, y0, w, h) = (b.origin.x, b.origin.y, b.size.width, b.size.height);
                let at = |(nx, ny): (f32, f32)| point(x0 + w * nx, y0 + h * ny);
                for tt in (30..=90).step_by(10) {
                    let x = at(curve_to_norm((tt as f32, 0.))).x.round();
                    window.paint_quad(fill(Bounds::new(point(x, y0), size(px(1.), h)), c(t.grid)));
                }
                for f in (20..=80).step_by(20) {
                    let y = at(curve_to_norm((20., f as f32))).y.round();
                    window.paint_quad(fill(Bounds::new(point(x0, y), size(w, px(1.))), c(t.grid)));
                }
                let line = |p: &[(f32, f32)]| {
                    let mut v = vec![at((0., p[0].1))];
                    v.extend(p.iter().map(|q| at(*q)));
                    v.push(at((1., p[p.len() - 1].1)));
                    v
                };
                if let Some(old) = &old {
                    let mut pb = PathBuilder::stroke(px(1.2)).dash_array(&[px(4.), px(3.)]);
                    pb.add_polygon(&line(old), false);
                    if let Ok(p) = pb.build() {
                        window.paint_path(p, ca(t.muted, 0.9));
                    }
                }
                let l = line(&pts);
                let mut fillp = PathBuilder::fill();
                fillp.add_polygon(&l, false);
                fillp.line_to(point(x0 + w, y0 + h));
                fillp.line_to(point(x0, y0 + h));
                fillp.close();
                if let Ok(p) = fillp.build() {
                    window.paint_path(p, ca(t.line, 0.1));
                }
                let stroke = if pending { t.accent } else { t.line };
                let mut pb = PathBuilder::stroke(px(2.));
                pb.add_polygon(&l, false);
                if let Ok(p) = pb.build() {
                    window.paint_path(p, c(stroke));
                }
                for (i, q) in pts.iter().enumerate() {
                    let p = at(*q);
                    let col = if hl == Some(i) || pending { t.accent } else { t.line };
                    window.paint_quad(quad(Bounds::new(point(p.x - px(4.), p.y - px(4.)), size(px(8.), px(8.))), px(0.), c(t.panel), px(1.8), c(col), BorderStyle::Solid));
                }
                // live marker
                let m = at(now);
                let mut pb = PathBuilder::stroke(px(1.)).dash_array(&[px(3.), px(3.)]);
                pb.move_to(point(m.x.round(), y0));
                pb.line_to(point(m.x.round(), y0 + h));
                if let Ok(p) = pb.build() {
                    window.paint_path(p, ca(t.text, 0.8));
                }
                window.paint_quad(quad(Bounds::new(point(m.x - px(4.), m.y - px(4.)), size(px(8.), px(8.))), px(4.), c(t.text), px(1.5), c(t.bg), BorderStyle::Solid));
                // Pointer capture while dragging a point: follow the pointer outside the plot too.
                if dragging {
                    let e1 = entity.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            e1.update(cx, |this, cx| {
                                if let Some(Drag::Curve(i)) = this.drag {
                                    let (nx, ny, _) = this.curve_norm(e.position);
                                    this.core.drag_curve_point(i, nx, ny);
                                    cx.notify();
                                }
                            });
                        }
                    });
                    let e2 = entity.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            e2.update(cx, |this, cx| {
                                this.drag = None;
                                cx.notify();
                            });
                        }
                    });
                }
            },
        )
        .size_full();

        let mut inner = div()
            .id("curve-plot")
            .relative()
            .size_full()
            .child(plot)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if let Some(i) = this.curve_hit(e.position) {
                        this.drag = Some(Drag::Curve(i));
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if this.drag.is_none() {
                    let h = this.curve_hit(e.position);
                    if h != this.curve_hover {
                        this.curve_hover = h;
                        cx.notify();
                    }
                }
            }))
            .when(self.curve_hover.is_some() || dragging, |d| d.cursor(CursorStyle::PointingHand));
        // axis labels
        for tt in (20..=100).step_by(20) {
            inner = inner.child(self.label(format!("{tt}°")).absolute().top(relative(1.)).mt(px(3.)).left(relative(curve_to_norm((tt as f32, 0.)).0)).ml(px(-8.)));
        }
        for f in [0, 50, 100] {
            inner = inner.child(self.label(format!("{f}%")).absolute().left(px(-30.)).top(relative(curve_to_norm((20., f as f32)).1)).mt(px(-6.)));
        }
        if let Some(i) = hl {
            let p = g.staged.curve[i];
            let q = curve_to_norm(p);
            inner = inner.child(
                self.mono()
                    .absolute()
                    .left(relative(q.0))
                    .top(relative(q.1))
                    .ml(px(8.))
                    .mt(px(-18.))
                    .text_size(px(10.))
                    .text_color(c(t.text))
                    .child(format!("{:.0}° → {:.0}%", p.0, p.1)),
            );
        }
        inner = inner.child(
            self.mono()
                .absolute()
                .top(px(2.))
                .left(relative(now.0))
                .ml(px(8.))
                .text_size(px(10.))
                .text_color(c(t.text))
                .child(format!("now {:.0}°C · fan {:.0}%", g.now.temp, g.now.fan)),
        );
        div()
            .relative()
            .flex_1()
            .h_full()
            .pl(px(30.))
            .pr(px(14.))
            .pt(px(8.))
            .pb(px(18.))
            .child(inner)
            .child(self.label("CURVE · TEMP → FAN").absolute().left(px(36.)).top(px(2.)).text_size(px(9.)))
    }

    fn tracks(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t();
        let g = self.core.gpu();
        let n = &g.now;
        let power_left = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(self.tk("POWER"))
            .child(self.big(format!("{:.0}", n.power), " W"))
            .child(self.ctlrow("limit", self.stepper("pl", format!("{} W", g.staged.power_limit), Step::PowerLimit, cx), g.power_pending().map(|p| format!("→ {p} W"))))
            .child(self.tk("OR DRAG THE ORANGE LINE").text_size(px(9.)));
        let clocks_left = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(self.tk("CLOCKS"))
            .child(self.big(format!("{:.0}", n.gclk), " MHz"))
            .child(self.ctlrow(
                "gfx",
                self.stepper("go", signed_mhz(g.staged.g_offset), Step::GOffset, cx),
                (g.staged.g_offset != g.applied.g_offset).then(|| format!("→ {}", signed_mhz(g.staged.g_offset))),
            ))
            .child(self.ctlrow(
                "mem",
                self.stepper("mo", signed_mhz(g.staged.m_offset), Step::MOffset, cx),
                (g.staged.m_offset != g.applied.m_offset).then(|| format!("→ {}", signed_mhz(g.staged.m_offset))),
            ));
        let mut chips = self.mono().flex().border_1().border_color(c(t.text)).text_size(px(10.));
        for (i, m) in FanMode::ALL.into_iter().enumerate() {
            let on = g.staged.fan_mode == m;
            chips = chips.child(
                div()
                    .id(("fan", i))
                    .px(px(6.))
                    .py(px(3.))
                    .when(i > 0, |d| d.border_l_1().border_color(c(t.text)))
                    .when(on, |d| d.bg(c(t.text)).text_color(c(t.panel)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.core.set_fan_mode(m);
                        cx.notify();
                    }))
                    .child(m.label()),
            );
        }
        let rpm = n.fan_rpm.map(|r| format!("{r} rpm")).unwrap_or("— rpm".into());
        let fans_left = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div().flex().child(self.tk("FANS")).child(div().flex_1()).child(
                    self.mono()
                        .id("toggle-curve")
                        .text_size(px(10.))
                        .text_color(c(t.accent))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.core.curve_open = !this.core.curve_open;
                            cx.notify();
                        }))
                        .child(if self.core.curve_open { "HIDE CURVE" } else { "EDIT CURVE" }),
                ),
            )
            .child(
                self.mono()
                    .flex()
                    .items_baseline()
                    .child(div().text_size(px(22.)).line_height(px(23.)).child(format!("{:.0}", n.fan)))
                    .child(div().text_size(px(12.)).text_color(c(t.muted)).child(format!("% · {rpm}"))),
            )
            .child(div().flex().child(chips))
            .when(g.cooling_paused, |d| d.child(self.mono().text_size(px(10.)).text_color(c(t.danger)).child("Fan curve paused after 3 failed fan writes")));
        let fan_chart = div()
            .flex()
            .flex_1()
            .gap(px(8.))
            .h_full()
            .when(self.core.curve_open, |d| d.child(div().flex_1().h_full().flex().child(self.curve_editor(cx))))
            .child(div().flex_1().h_full().flex().child(self.chart(Chart::Fan, Metric::Fan, false, Some("FAN SPEED"), false, cx)));

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_hidden()
            .child(self.track(Some(84.), div().flex().flex_col().gap(px(4.)).child(self.tk("TEMPERATURE")).child(self.big(format!("{:.0}", n.temp), "°C")), self.chart(Chart::Temp, Metric::Temp, false, None, false, cx), Some(Metric::Temp)))
            .child(self.track(Some(84.), div().flex().flex_col().gap(px(4.)).child(self.tk("UTILIZATION")).child(self.big(format!("{:.0}", n.util), "%")), self.chart(Chart::Util, Metric::Util, true, None, false, cx), Some(Metric::Util)))
            .child(self.track(Some(112.), power_left, self.chart(Chart::Power, Metric::Power, false, None, true, cx), Some(Metric::Power)))
            .child(self.track(Some(112.), clocks_left, self.chart(Chart::Clocks, Metric::Gclk, false, Some("GRAPHICS"), false, cx), Some(Metric::Gclk)))
            .child(self.track(if self.core.curve_open { None } else { Some(96.) }, fans_left, fan_chart, Some(Metric::Fan)))
    }

    fn statusbar(&self) -> impl IntoElement {
        let t = self.t();
        let held = self.core.gpu().pending_count() > 0;
        self.mono()
            .h(px(24.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(12.))
            .bg(c(t.chrome))
            .text_color(c(t.chrome_muted))
            .text_size(px(10.))
            .child(div().flex().items_center().child(div().size(px(6.)).mr(px(6.)).bg(c(t.ok))).child(self.core.status_daemon()))
            .child("AUTO: CYBERPUNK 2077 → QUIET")
            .when(held, |d| d.child(div().text_color(c(t.accent)).child("EDIT HOLD")))
            .child(div().flex_1())
            .when(self.core.problems() > 0, |d| d.child(div().text_color(c(t.accent)).child(format!("▲ {} problem", self.core.problems()))))
    }

    fn palette_view(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let p = self.palette.as_ref()?;
        let t = self.t();
        let q = p.input.read(cx).value().to_string();
        let cmds = self.core.commands(&q);
        let mut list = div().flex().flex_col().max_h(px(300.)).overflow_hidden();
        for (i, cmd) in cmds.iter().enumerate().take(12) {
            let on = i == p.sel;
            list = list.child(
                div()
                    .id(("cmd", i))
                    .px(px(10.))
                    .py(px(5.))
                    .when(on, |d| d.bg(c(t.text)).text_color(c(t.panel)))
                    .when(!on, |d| d.hover(|s| s.bg(c(t.hover))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(p) = &mut this.palette {
                            p.sel = i;
                        }
                        this.run_palette(window, cx);
                    }))
                    .child(cmd.label.clone()),
            );
        }
        Some(
            div()
                .absolute()
                .top(px(60.))
                .left(relative(0.5))
                .ml(px(-260.))
                .w(px(520.))
                .bg(c(t.panel2))
                .border_1()
                .border_color(c(t.text))
                .shadow(vec![BoxShadow::new(px(6.), px(6.), ca(t.text, 0.9))])
                .text_color(c(t.text))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .capture_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    let n = this.core.commands(&this.palette_query(cx)).len();
                    if let Some(p) = &mut this.palette {
                        match e.keystroke.key.as_str() {
                            "down" => p.sel = (p.sel + 1).min(n.saturating_sub(1)),
                            "up" => p.sel = p.sel.saturating_sub(1),
                            _ => return,
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .child(div().p(px(6.)).border_b_1().border_color(c(t.border)).child(Input::new(&p.input).appearance(false)))
                .child(list),
        )
    }
}

impl Render for Prism {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames.set(self.frames.get() + 1);
        let t = *self.t();
        sync_component_theme(&t, self.core.dark(), cx);
        let decorations = window.window_decorations();
        let tiled = match decorations {
            Decorations::Server => true,
            Decorations::Client { tiling } => tiling.is_tiled(),
        };
        window.set_client_inset(if tiled { px(0.) } else { px(INSET) });
        let main = div()
            .flex_1()
            .flex()
            .flex_col()
            .min_w_0()
            .bg(c(t.panel))
            .child(self.header(cx))
            .child(self.apply_strip(cx))
            .child(self.tracks(cx));
        let win = div()
            .id("win")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(c(t.bg))
            .text_color(c(t.text))
            .font_family(FONT_UI)
            .text_size(px(12.))
            .when(!tiled, |d| d.rounded(px(6.)).border_1().border_color(c(t.chrome)).shadow(vec![BoxShadow::new(px(0.), px(2.), hsla(0., 0., 0., 0.45)).blur_radius(px(INSET * 0.75))]))
            .child(self.titlebar(window, cx))
            .child(div().flex_1().flex().min_h_0().child(self.sidebar(cx)).child(main))
            .child(self.statusbar())
            .children(self.palette_view(cx));

        div()
            .id("backdrop")
            .size_full()
            .track_focus(&self.focus)
            .key_context("Prism")
            .on_action(cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)))
            .on_action(cx.listener(|this, _: &ApplyEdits, _, cx| {
                this.core.apply();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.palette.is_some() {
                    this.close_palette(window, cx);
                } else {
                    this.core.discard();
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .when(!tiled, |d| {
                d.p(px(INSET))
                    .child(
                        canvas(
                            |_b, window, _| window.insert_hitbox(Bounds::new(point(px(0.), px(0.)), window.window_bounds().get_bounds().size), HitboxBehavior::Normal),
                            move |_b, hitbox, window, _| {
                                let size = window.window_bounds().get_bounds().size;
                                if let Some(edge) = resize_edge(window.mouse_position(), px(INSET), size) {
                                    window.set_cursor_style(
                                        match edge {
                                            ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                                            ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                                            ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
                                            ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
                                        },
                                        &hitbox,
                                    );
                                }
                            },
                        )
                        .size_full()
                        .absolute(),
                    )
                    .on_mouse_down(MouseButton::Left, |e, window, _| {
                        let size = window.window_bounds().get_bounds().size;
                        if let Some(edge) = resize_edge(e.position, px(INSET), size) {
                            window.start_window_resize(edge);
                        }
                    })
            })
            .child(win)
    }
}

/// Scripted pointer input through gpui's real dispatch path, while the 1 Hz
/// updates keep redrawing: drag the power-limit line, then a fan-curve point.
async fn selftest(this: WeakEntity<Prism>, cx: &mut AsyncWindowContext) {
    let ex = cx.background_executor().clone();
    let wait = |ms| ex.timer(Duration::from_millis(ms));
    let delay: u64 = std::env::var("SPIKE_SELFTEST_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let loops: usize = std::env::var("SPIKE_SELFTEST_LOOPS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    wait(delay * 1000).await;
    let Ok((pb, cb, curve)) = this.read_with(cx, |p, _| {
        (p.chart_bounds[Chart::Power as usize].get(), p.curve_bounds.get(), p.core.gpu().staged.curve.clone())
    }) else {
        return;
    };
    let send = |cx: &mut AsyncWindowContext, ev: PlatformInput| {
        let _ = cx.update(|window, cx| window.dispatch_event(ev, cx));
    };
    let down = |p| PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: p, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    let mv = |p| PlatformInput::MouseMove(MouseMoveEvent { position: p, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() });
    let up = |p| PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: p, modifiers: Modifiers::default(), click_count: 1 });
    for _ in 0..loops {
        // 1. power line: press mid-chart, drag up over 1.5 s (crossing a 1 Hz tick)
        let x = pb.origin.x + pb.size.width * 0.6;
        let mut p = point(x, pb.origin.y + pb.size.height * 0.45);
        send(cx, down(p));
        for i in 0..30 {
            p.y = pb.origin.y + pb.size.height * (0.45 - 0.004 * i as f32);
            send(cx, mv(p));
            wait(50).await;
        }
        send(cx, up(p));
        let _ = this.read_with(cx, |p, _| eprintln!("[selftest] power: applied={} staged={} pending={}", p.core.gpu().applied.power_limit, p.core.gpu().staged.power_limit, p.core.gpu().pending_count()));
        // 2. curve point 2: press on it, drag up-left, ending outside the plot
        let (nx, ny) = curve_to_norm(this.read_with(cx, |p, _| p.core.gpu().staged.curve[2]).unwrap_or(curve[2]));
        let mut p = point(cb.origin.x + cb.size.width * nx, cb.origin.y + cb.size.height * ny);
        send(cx, PlatformInput::MouseMove(MouseMoveEvent { position: p, pressed_button: None, modifiers: Modifiers::default() }));
        send(cx, down(p));
        for _ in 0..30 {
            p.y -= px(3.);
            p.x -= px(1.);
            send(cx, mv(p));
            wait(50).await;
        }
        send(cx, up(p));
        let _ = this.read_with(cx, |p, _| eprintln!("[selftest] curve: before={:?} after={:?} pending={}", curve[2], p.core.gpu().staged.curve[2], p.core.gpu().pending_count()));
        let _ = this.update(cx, |p, _| p.core.discard());
    }
    // 3. text input: Ctrl+K, type "perf", Enter -> applies the PERFORMANCE profile
    let key = |k: &str, ctrl: bool, ch: Option<&str>| {
        PlatformInput::KeyDown(KeyDownEvent {
            keystroke: Keystroke { modifiers: Modifiers { control: ctrl, ..Default::default() }, key: k.into(), key_char: ch.map(|c| c.into()) },
            is_held: false,
            prefer_character_input: false,
        })
    };
    let _ = key;
    let stroke = |cx: &mut AsyncWindowContext, k: &str| {
        let _ = cx.update(|window, cx| window.dispatch_keystroke(Keystroke::parse(k).unwrap(), cx));
    };
    stroke(cx, "ctrl-k");
    wait(300).await;
    for ch in ["p", "e", "r", "f"] {
        stroke(cx, ch);
        wait(120).await;
    }
    let _ = this.read_with(cx, |p, cx| eprintln!("[selftest] palette query={:?} matches={}", p.palette_query(cx), p.core.commands(&p.palette_query(cx)).len()));
    wait(800).await;
    stroke(cx, "enter");
    wait(300).await;
    let _ = this.read_with(cx, |p, _| eprintln!("[selftest] palette: profile={} open={}", p.core.gpu().profiles[p.core.gpu().profile].name, p.palette.is_some()));
    eprintln!("[selftest] done");
}

fn resize_edge(pos: Point<Pixels>, s: Pixels, size: Size<Pixels>) -> Option<ResizeEdge> {
    let (l, r, t, b) = (pos.x < s, pos.x > size.width - s, pos.y < s, pos.y > size.height - s);
    Some(match (l, r, t, b) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (_, true, true, _) => ResizeEdge::TopRight,
        (true, _, _, true) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (_, _, true, _) => ResizeEdge::Top,
        (_, _, _, true) => ResizeEdge::Bottom,
        (true, ..) => ResizeEdge::Left,
        (_, true, ..) => ResizeEdge::Right,
        _ => return None,
    })
}

/// gpui-component widgets (the palette's text input) read their own theme;
/// point it at C's tokens so the one token table drives everything.
fn sync_component_theme(t: &Tokens, dark: bool, cx: &mut App) {
    use gpui_kit::component::theme::{Theme, ThemeMode};
    let mode = if dark { ThemeMode::Dark } else { ThemeMode::Light };
    if Theme::global(cx).mode != mode {
        Theme::change(mode, None, cx);
    }
    let th = Theme::global_mut(cx);
    th.font_family = FONT_MONO.into();
    th.mono_font_family = FONT_MONO.into();
    th.font_size = px(12.);
    th.radius = px(0.);
    th.colors.background = c(t.panel2);
    th.colors.foreground = c(t.text);
    th.colors.border = c(t.border);
    th.colors.input = c(t.border);
    th.colors.ring = c(t.accent);
    th.colors.caret = c(t.accent);
    th.colors.selection = ca(t.accent, 0.3);
    th.colors.muted_foreground = c(t.muted);
}

fn main() {
    gpui_kit::application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../fonts/Inter-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../../fonts/Inter-SemiBold.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../../fonts/JetBrainsMono-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../../fonts/JetBrainsMono-Bold.ttf").as_slice()),
            ])
            .expect("fonts");
        cx.bind_keys([
            KeyBinding::new("ctrl-k", OpenPalette, Some("Prism")),
            KeyBinding::new("ctrl-enter", ApplyEdits, Some("Prism")),
            KeyBinding::new("escape", Escape, Some("Prism")),
        ]);
        let bounds = Bounds::centered(None, size(px(1280. + 2. * INSET), px(800. + 2. * INSET)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: WindowBackgroundAppearance::Transparent,
                window_decorations: Some(WindowDecorations::Client),
                window_min_size: Some(size(px(960.), px(600.))),
                app_id: Some("dev.prism.spike.gpui".into()),
                titlebar: Some(TitlebarOptions { title: Some("Prism (gpui spike)".into()), ..Default::default() }),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Prism::new(window, cx)),
        )
        .expect("window");
        cx.activate(true);
    });
}
