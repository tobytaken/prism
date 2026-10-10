//! Design tokens for direction C · Tracks, defined once. Views read these and
//! never hard-code colours, so a later theming effort only swaps this table.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub bg: u32,
    pub panel: u32,
    pub panel2: u32,
    pub border: u32,
    pub hover: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub danger: u32,
    pub ok: u32,
    pub grid: u32,
    pub line: u32,
    pub chrome: u32,
    pub chrome_text: u32,
    pub chrome_muted: u32,
    pub chrome_border: u32,
    pub chrome_hover: u32,
    pub chrome_hi: u32,
    /// Text on the accent fill (the pending change strip).
    pub on_accent: u32,
}

pub const LIGHT: Tokens = Tokens {
    bg: 0xe9e6df,
    panel: 0xf6f4ef,
    panel2: 0xffffff,
    border: 0xd2cec4,
    hover: 0xe2ded5,
    text: 0x1c1b19,
    muted: 0x77736a,
    accent: 0xff5a1f,
    danger: 0xd93b2b,
    ok: 0x2f9e5a,
    grid: 0xe3dfd6,
    line: 0x1c1b19,
    chrome: 0x1c1b19,
    chrome_text: 0xe9e6df,
    chrome_muted: 0xa8a399,
    chrome_border: 0x4a4843,
    chrome_hover: 0x34322e,
    chrome_hi: 0xffffff,
    on_accent: 0xffffff,
};

pub const DARK: Tokens = Tokens {
    bg: 0x121110,
    panel: 0x1a1917,
    panel2: 0x22201d,
    border: 0x33302b,
    hover: 0x292723,
    text: 0xece8df,
    muted: 0x8e897f,
    accent: 0xff6326,
    danger: 0xff5a48,
    ok: 0x4cc27a,
    grid: 0x262420,
    line: 0xece8df,
    chrome: 0x0a0a09,
    chrome_text: 0xece8df,
    chrome_muted: 0x8e897f,
    chrome_border: 0x3a3732,
    chrome_hover: 0x23211e,
    chrome_hi: 0xffffff,
    on_accent: 0xffffff,
};

pub fn tokens(dark: bool) -> &'static Tokens {
    if dark { &DARK } else { &LIGHT }
}

pub const FONT_UI: &str = "Inter";
pub const FONT_MONO: &str = "JetBrains Mono";
