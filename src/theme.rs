//! Colors. The palette is defined once in 24-bit RGB and downsampled to
//! whatever the terminal can show: truecolor, the xterm 256-color cube,
//! the 16 basic ANSI colors, or nothing at all (`NO_COLOR`).

use std::env;

use crossterm::style::Color;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

impl ColorDepth {
    /// Best guess at what the terminal supports. `ZENCODE_COLOR` overrides
    /// (`truecolor`, `256`, `16`, `none`).
    pub fn detect() -> Self {
        if let Ok(v) = env::var("ZENCODE_COLOR") {
            match v.to_ascii_lowercase().as_str() {
                "truecolor" | "24bit" | "24-bit" => return Self::TrueColor,
                "256" => return Self::Ansi256,
                "16" => return Self::Ansi16,
                "none" | "mono" | "0" => return Self::Mono,
                _ => {}
            }
        }
        if env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return Self::Mono;
        }

        let colorterm = env::var("COLORTERM")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return Self::TrueColor;
        }
        // Windows Terminal doesn't set COLORTERM but supports 24-bit color.
        if env::var_os("WT_SESSION").is_some() {
            return Self::TrueColor;
        }
        let term_program = env::var("TERM_PROGRAM").unwrap_or_default();
        if matches!(
            term_program.as_str(),
            "iTerm.app" | "WezTerm" | "vscode" | "ghostty" | "Hyper" | "Tabby"
        ) {
            return Self::TrueColor;
        }

        let term = env::var("TERM").unwrap_or_default();
        if term.contains("truecolor") || term.contains("direct") || term.contains("kitty") {
            return Self::TrueColor;
        }
        if term.contains("256") {
            return Self::Ansi256;
        }
        if cfg!(windows) {
            // Modern conhost handles 256 colors via VT sequences.
            return Self::Ansi256;
        }
        if term == "dumb" {
            return Self::Mono;
        }
        Self::Ansi16
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorDepth::TrueColor => "24-bit",
            ColorDepth::Ansi256 => "256",
            ColorDepth::Ansi16 => "16",
            ColorDepth::Mono => "mono",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(
            mix(self.0, other.0),
            mix(self.1, other.1),
            mix(self.2, other.2),
        )
    }
}

pub struct Theme {
    pub depth: ColorDepth,

    pub bg: Rgb,
    pub fg: Rgb,
    pub cur_line_bg: Rgb,

    pub gutter_bg: Rgb,
    pub gutter_fg: Rgb,
    pub gutter_cur_fg: Rgb,

    pub title_from: Rgb,
    pub title_to: Rgb,
    pub title_fg: Rgb,

    pub status_fg: Rgb,
    pub error_fg: Rgb,
    pub prompt_fg: Rgb,

    pub bar_bg: Rgb,
    pub key_bg: Rgb,
    pub key_fg: Rgb,
    pub label_fg: Rgb,

    pub select_bg: Rgb,
    pub match_bg: Rgb,
    pub match_fg: Rgb,
    pub badge_bg: Rgb,
    pub badge_fg: Rgb,
    /// Background tint for U+3000 IDEOGRAPHIC SPACE, so 全角スペース is visible.
    pub zenkaku_space_bg: Rgb,
    pub tab_fg: Rgb,
    pub control_fg: Rgb,
    pub edge_fg: Rgb,
}

impl Theme {
    /// "Sakura night": dark plum background with pink-to-violet accents.
    pub fn sakura(depth: ColorDepth) -> Self {
        Self {
            depth,
            bg: Rgb(24, 22, 34),
            fg: Rgb(224, 219, 240),
            cur_line_bg: Rgb(38, 35, 56),

            gutter_bg: Rgb(30, 28, 44),
            gutter_fg: Rgb(92, 87, 124),
            gutter_cur_fg: Rgb(255, 160, 200),

            title_from: Rgb(255, 121, 176),
            title_to: Rgb(130, 120, 255),
            title_fg: Rgb(22, 18, 32),

            status_fg: Rgb(255, 205, 125),
            error_fg: Rgb(255, 110, 120),
            prompt_fg: Rgb(120, 225, 205),

            bar_bg: Rgb(30, 28, 44),
            key_bg: Rgb(150, 130, 255),
            key_fg: Rgb(22, 18, 32),
            label_fg: Rgb(185, 180, 212),

            select_bg: Rgb(86, 70, 140),
            match_bg: Rgb(255, 205, 125),
            match_fg: Rgb(22, 18, 32),
            badge_bg: Rgb(22, 18, 32),
            badge_fg: Rgb(255, 205, 125),
            zenkaku_space_bg: Rgb(58, 52, 86),
            tab_fg: Rgb(60, 56, 86),
            control_fg: Rgb(255, 110, 120),
            edge_fg: Rgb(255, 121, 176),
        }
    }

    pub fn mono(&self) -> bool {
        self.depth == ColorDepth::Mono
    }

    pub fn color(&self, c: Rgb) -> Color {
        match self.depth {
            ColorDepth::TrueColor => Color::Rgb {
                r: c.0,
                g: c.1,
                b: c.2,
            },
            ColorDepth::Ansi256 => Color::AnsiValue(to_ansi256(c)),
            ColorDepth::Ansi16 => to_ansi16(c),
            ColorDepth::Mono => Color::Reset,
        }
    }
}

/// Nearest entry in the xterm 6x6x6 cube or the 24-step grayscale ramp.
fn to_ansi256(Rgb(r, g, b): Rgb) -> u8 {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |v: u8| {
        (0..6)
            .min_by_key(|&i| (STEPS[i] as i32 - v as i32).abs())
            .unwrap() as u8
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = (STEPS[ri as usize], STEPS[gi as usize], STEPS[bi as usize]);
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;

    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gray_i = ((avg.saturating_sub(8)) / 10).min(23) as u8;
    let gray_v = 8 + 10 * gray_i;
    let gray_idx = 232 + gray_i;

    let dist = |(cr, cg, cb): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(r, cr) + d(g, cg) + d(b, cb)
    };
    if dist((gray_v, gray_v, gray_v)) < dist(cube) {
        gray_idx
    } else {
        cube_idx
    }
}

/// Nearest of the 16 basic ANSI colors (using typical xterm values).
fn to_ansi16(Rgb(r, g, b): Rgb) -> Color {
    const TABLE: [(Color, (u8, u8, u8)); 16] = [
        (Color::Black, (0, 0, 0)),
        (Color::DarkRed, (205, 0, 0)),
        (Color::DarkGreen, (0, 205, 0)),
        (Color::DarkYellow, (205, 205, 0)),
        (Color::DarkBlue, (0, 0, 238)),
        (Color::DarkMagenta, (205, 0, 205)),
        (Color::DarkCyan, (0, 205, 205)),
        (Color::Grey, (229, 229, 229)),
        (Color::DarkGrey, (127, 127, 127)),
        (Color::Red, (255, 0, 0)),
        (Color::Green, (0, 255, 0)),
        (Color::Yellow, (255, 255, 0)),
        (Color::Blue, (92, 92, 255)),
        (Color::Magenta, (255, 0, 255)),
        (Color::Cyan, (0, 255, 255)),
        (Color::White, (255, 255, 255)),
    ];
    let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
    TABLE
        .iter()
        .min_by_key(|(_, (cr, cg, cb))| d(r, *cr) + d(g, *cg) + d(b, *cb))
        .map(|(c, _)| *c)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi256_maps_primaries() {
        assert_eq!(to_ansi256(Rgb(255, 0, 0)), 196);
        assert_eq!(to_ansi256(Rgb(0, 0, 0)), 16);
        assert_eq!(to_ansi256(Rgb(128, 128, 128)), 244);
    }

    #[test]
    fn ansi16_maps_dark_background_to_black() {
        assert_eq!(to_ansi16(Rgb(24, 22, 34)), Color::Black);
    }
}
