//! Colors (Monaco `base/common/color.ts`) and the default document colors (`defaultDocumentColorsComputer.ts`), ported (MIT).
//!
//! The arithmetic follows the TypeScript exactly, including JavaScript's
//! `Math.round` (ties round up), `| 0` truncation, `roundFloat(x, 3)` on
//! every non-integer component and `toFixed` (ties round away from zero).
//!
//! Deviations from Monaco:
//! - [`Color::parse`] returns `None` for a malformed `rgb(...)`/`rgba(...)`
//!   instead of throwing.
//! - [`Color::flatten`] with no backgrounds returns `self` (Monaco throws).
//! - [`compute_default_document_colors`] takes the text directly and returns
//!   byte ranges into it rather than 1-based line/column `IRange`s. A match
//!   that spans lines (`rgb(` followed by a newline) gets its true range.
//! - JavaScript regex semantics are kept: `\s` is JavaScript's whitespace
//!   set, `\d` is ASCII and `\b` is an ASCII word boundary. The hex lookbehind
//!   `(?<=['"\s])` is emulated by consuming the preceding character outside
//!   the reported range.

use std::fmt;
use std::ops::Range;
use std::sync::LazyLock;

use regex::{Captures, Regex};

/// JavaScript `Math.round`: rounds half-way cases towards `+∞`.
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// JavaScript `Math.min` for two numbers (NaN wins).
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// JavaScript `Math.max` for two numbers (NaN wins).
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// JavaScript `x | 0` for values already clamped to a small range.
fn js_trunc(x: f64) -> f64 {
    if x.is_nan() { 0.0 } else { x.trunc() }
}

/// Monaco `roundFloat`.
fn round_float(number: f64, decimal_points: i32) -> f64 {
    let decimal = 10f64.powi(decimal_points);
    js_round(number * decimal) / decimal
}

/// Clamps to `[0, 1]` and rounds to 3 decimals, as the TS constructors do.
fn unit3(x: f64) -> f64 {
    round_float(js_max(js_min(1.0, x), 0.0), 3)
}

/// JavaScript `Number.prototype.toFixed` for finite numbers below `1e21`:
/// the exact decimal value rounded half away from zero.
fn js_to_fixed(x: f64, digits: usize) -> String {
    // Every finite f64 has a finite decimal expansion of at most 1074
    // fractional digits, so this is exact.
    let exact = format!("{:.1074}", x.abs());
    let (int_part, frac_part) = exact.split_once('.').unwrap_or((&exact, ""));
    let mut kept: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes().take(digits))
        .collect();
    if frac_part.as_bytes().get(digits).is_some_and(|&d| d >= b'5') {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let split = kept.len() - digits;
    let mut out = String::new();
    if x < 0.0 && kept.iter().any(|&d| d != b'0') {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&kept[..split]).unwrap_or("0"));
    if digits > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(&kept[split..]).unwrap_or(""));
    }
    out
}

/// An RGB color with alpha (Monaco `RGBA`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    /// Red, in `0..=255`.
    pub r: u8,
    /// Green, in `0..=255`.
    pub g: u8,
    /// Blue, in `0..=255`.
    pub b: u8,
    /// Alpha, in `[0, 1]`, rounded to 3 decimals.
    pub a: f64,
}

impl Rgba {
    /// An RGBA value; alpha is clamped to `[0, 1]` and rounded to 3
    /// decimals (Monaco `new RGBA(r, g, b, a)`).
    pub fn new(r: u8, g: u8, b: u8, a: f64) -> Self {
        Self {
            r,
            g,
            b,
            a: unit3(a),
        }
    }

    /// Monaco `new RGBA(r, g, b, a)` for arbitrary numbers: the channels are
    /// clamped to `[0, 255]` and truncated (NaN becomes 0), and alpha is
    /// clamped and rounded to 3 decimals.
    pub fn new_clamped(r: f64, g: f64, b: f64, a: f64) -> Self {
        let channel = |c: f64| js_trunc(js_min(255.0, js_max(0.0, c))) as u8;
        Self {
            r: channel(r),
            g: channel(g),
            b: channel(b),
            a: unit3(a),
        }
    }
}

/// A hue/saturation/luminosity color with alpha (Monaco `HSLA`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsla {
    /// Hue, an integer in `[0, 360]`.
    pub h: f64,
    /// Saturation, in `[0, 1]`, rounded to 3 decimals.
    pub s: f64,
    /// Luminosity, in `[0, 1]`, rounded to 3 decimals.
    pub l: f64,
    /// Alpha, in `[0, 1]`, rounded to 3 decimals.
    pub a: f64,
}

impl Hsla {
    /// Clamps the components, truncates the hue and rounds the rest to 3
    /// decimals (Monaco `new HSLA(h, s, l, a)`).
    pub fn new(h: f64, s: f64, l: f64, a: f64) -> Self {
        Self {
            h: js_trunc(js_max(js_min(360.0, h), 0.0)),
            s: unit3(s),
            l: unit3(l),
            a: unit3(a),
        }
    }

    /// Converts RGB to HSL (Monaco `HSLA.fromRGBA`).
    pub fn from_rgba(rgba: Rgba) -> Self {
        let r = f64::from(rgba.r) / 255.0;
        let g = f64::from(rgba.g) / 255.0;
        let b = f64::from(rgba.b) / 255.0;
        let a = rgba.a;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let mut h = 0.0;
        let mut s = 0.0;
        let l = (min + max) / 2.0;
        let chroma = max - min;

        if chroma > 0.0 {
            s = (if l <= 0.5 {
                chroma / (2.0 * l)
            } else {
                chroma / (2.0 - (2.0 * l))
            })
            .min(1.0);

            if max == r {
                h = (g - b) / chroma + if g < b { 6.0 } else { 0.0 };
            } else if max == g {
                h = (b - r) / chroma + 2.0;
            } else if max == b {
                h = (r - g) / chroma + 4.0;
            }

            h *= 60.0;
            h = js_round(h);
        }
        Self::new(h, s, l, a)
    }

    /// Monaco `HSLA._hue2rgb`.
    fn hue2rgb(p: f64, q: f64, mut t: f64) -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    }

    /// Converts HSL to RGB (Monaco `HSLA.toRGBA`).
    pub fn to_rgba(self) -> Rgba {
        let h = self.h / 360.0;
        let Self { s, l, a, .. } = self;
        let (r, g, b);

        if s == 0.0 {
            r = l;
            g = l;
            b = l;
        } else {
            let q = if l < 0.5 {
                l * (1.0 + s)
            } else {
                l + s - l * s
            };
            let p = 2.0 * l - q;
            r = Self::hue2rgb(p, q, h + 1.0 / 3.0);
            g = Self::hue2rgb(p, q, h);
            b = Self::hue2rgb(p, q, h - 1.0 / 3.0);
        }

        Rgba::new_clamped(
            js_round(r * 255.0),
            js_round(g * 255.0),
            js_round(b * 255.0),
            a,
        )
    }
}

/// A hue/saturation/value color with alpha (Monaco `HSVA`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsva {
    /// Hue, an integer in `[0, 360]`.
    pub h: f64,
    /// Saturation, in `[0, 1]`, rounded to 3 decimals.
    pub s: f64,
    /// Value, in `[0, 1]`, rounded to 3 decimals.
    pub v: f64,
    /// Alpha, in `[0, 1]`, rounded to 3 decimals.
    pub a: f64,
}

impl Hsva {
    /// Clamps the components, truncates the hue and rounds the rest to 3
    /// decimals (Monaco `new HSVA(h, s, v, a)`).
    pub fn new(h: f64, s: f64, v: f64, a: f64) -> Self {
        Self {
            h: js_trunc(js_max(js_min(360.0, h), 0.0)),
            s: unit3(s),
            v: unit3(v),
            a: unit3(a),
        }
    }

    /// Converts RGB to HSV (Monaco `HSVA.fromRGBA`).
    pub fn from_rgba(rgba: Rgba) -> Self {
        let r = f64::from(rgba.r) / 255.0;
        let g = f64::from(rgba.g) / 255.0;
        let b = f64::from(rgba.b) / 255.0;
        let cmax = r.max(g).max(b);
        let cmin = r.min(g).min(b);
        let delta = cmax - cmin;
        let s = if cmax == 0.0 { 0.0 } else { delta / cmax };

        let m = if delta == 0.0 {
            0.0
        } else if cmax == r {
            ((((g - b) / delta) % 6.0) + 6.0) % 6.0
        } else if cmax == g {
            ((b - r) / delta) + 2.0
        } else {
            ((r - g) / delta) + 4.0
        };

        Self::new(js_round(m * 60.0), s, cmax, rgba.a)
    }

    /// Converts HSV to RGB (Monaco `HSVA.toRGBA`).
    pub fn to_rgba(self) -> Rgba {
        let Self { h, s, v, a } = self;
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;
        let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);

        if h < 60.0 {
            r = c;
            g = x;
        } else if h < 120.0 {
            r = x;
            g = c;
        } else if h < 180.0 {
            g = c;
            b = x;
        } else if h < 240.0 {
            g = x;
            b = c;
        } else if h < 300.0 {
            r = x;
            b = c;
        } else if h <= 360.0 {
            r = c;
            b = x;
        }

        Rgba::new_clamped(
            js_round((r + m) * 255.0),
            js_round((g + m) * 255.0),
            js_round((b + m) * 255.0),
            a,
        )
    }
}

/// A color (Monaco `Color`).
///
/// Like Monaco, a color built from [`Hsla`] or [`Hsva`] remembers that
/// value and returns it from [`Color::hsla`] / [`Color::hsva`] instead of
/// recomputing it from the (rounded) RGB channels.
#[derive(Clone, Copy, Debug)]
pub struct Color {
    rgba: Rgba,
    hsla: Option<Hsla>,
    hsva: Option<Hsva>,
}

impl Color {
    const fn opaque_const(r: u8, g: u8, b: u8) -> Self {
        Self {
            rgba: Rgba { r, g, b, a: 1.0 },
            hsla: None,
            hsva: None,
        }
    }

    /// Opaque white (Monaco `Color.white`).
    pub const WHITE: Self = Self::opaque_const(255, 255, 255);
    /// Opaque black (Monaco `Color.black`).
    pub const BLACK: Self = Self::opaque_const(0, 0, 0);
    /// Opaque red (Monaco `Color.red`).
    pub const RED: Self = Self::opaque_const(255, 0, 0);
    /// Opaque blue (Monaco `Color.blue`).
    pub const BLUE: Self = Self::opaque_const(0, 0, 255);
    /// Opaque green, `#00ff00` (Monaco `Color.green`).
    pub const GREEN: Self = Self::opaque_const(0, 255, 0);
    /// Opaque cyan (Monaco `Color.cyan`).
    pub const CYAN: Self = Self::opaque_const(0, 255, 255);
    /// Opaque light grey (Monaco `Color.lightgrey`).
    pub const LIGHTGREY: Self = Self::opaque_const(211, 211, 211);
    /// Fully transparent black (Monaco `Color.transparent`).
    pub const TRANSPARENT: Self = Self {
        rgba: Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 0.0,
        },
        hsla: None,
        hsva: None,
    };

    /// A color from RGBA (Monaco `new Color(rgba)`).
    pub fn new(rgba: Rgba) -> Self {
        Self {
            rgba,
            hsla: None,
            hsva: None,
        }
    }

    /// A color from HSLA, converted to RGBA (Monaco `new Color(hsla)`).
    pub fn from_hsla(hsla: Hsla) -> Self {
        Self {
            rgba: hsla.to_rgba(),
            hsla: Some(hsla),
            hsva: None,
        }
    }

    /// A color from HSVA, converted to RGBA (Monaco `new Color(hsva)`).
    pub fn from_hsva(hsva: Hsva) -> Self {
        Self {
            rgba: hsva.to_rgba(),
            hsla: None,
            hsva: Some(hsva),
        }
    }

    /// Parses `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`, falling back to
    /// [`Color::RED`] (Monaco `Color.fromHex`).
    pub fn from_hex(hex: &str) -> Self {
        Color::parse_hex(hex).unwrap_or(Self::RED)
    }

    /// The RGBA value (Monaco `Color.rgba`).
    pub fn rgba(&self) -> Rgba {
        self.rgba
    }

    /// The HSLA value (Monaco `Color.hsla`).
    pub fn hsla(&self) -> Hsla {
        self.hsla.unwrap_or_else(|| Hsla::from_rgba(self.rgba))
    }

    /// The HSVA value (Monaco `Color.hsva`).
    pub fn hsva(&self) -> Hsva {
        self.hsva.unwrap_or_else(|| Hsva::from_rgba(self.rgba))
    }

    /// Whether the RGBA, HSLA and HSVA values all match (Monaco
    /// `Color.equals`).
    pub fn equals(&self, other: &Color) -> bool {
        self.rgba == other.rgba && self.hsla() == other.hsla() && self.hsva() == other.hsva()
    }

    /// WCAG relative luminance in `[0, 1]`, rounded to 4 decimals (Monaco
    /// `Color.getRelativeLuminance`).
    pub fn get_relative_luminance(&self) -> f64 {
        let r = Self::relative_luminance_for_component(self.rgba.r);
        let g = Self::relative_luminance_for_component(self.rgba.g);
        let b = Self::relative_luminance_for_component(self.rgba.b);
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        round_float(luminance, 4)
    }

    /// Monaco `Color._relativeLuminanceForComponent`.
    fn relative_luminance_for_component(color: u8) -> f64 {
        let c = f64::from(color) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// WCAG contrast ratio in `[1, 21]` (Monaco `Color.getContrastRatio`).
    pub fn get_contrast_ratio(&self, another: &Color) -> f64 {
        let lum1 = self.get_relative_luminance();
        let lum2 = another.get_relative_luminance();
        if lum1 > lum2 {
            (lum1 + 0.05) / (lum2 + 0.05)
        } else {
            (lum2 + 0.05) / (lum1 + 0.05)
        }
    }

    fn yiq(&self) -> f64 {
        (f64::from(self.rgba.r) * 299.0
            + f64::from(self.rgba.g) * 587.0
            + f64::from(self.rgba.b) * 114.0)
            / 1000.0
    }

    /// Whether the YIQ brightness is below 128 (Monaco `Color.isDarker`).
    pub fn is_darker(&self) -> bool {
        self.yiq() < 128.0
    }

    /// Whether the YIQ brightness is at least 128 (Monaco `Color.isLighter`).
    pub fn is_lighter(&self) -> bool {
        self.yiq() >= 128.0
    }

    /// Whether this has a higher relative luminance (Monaco
    /// `Color.isLighterThan`).
    pub fn is_lighter_than(&self, another: &Color) -> bool {
        self.get_relative_luminance() > another.get_relative_luminance()
    }

    /// Whether this has a lower relative luminance (Monaco
    /// `Color.isDarkerThan`).
    pub fn is_darker_than(&self, another: &Color) -> bool {
        self.get_relative_luminance() < another.get_relative_luminance()
    }

    /// Scales HSL luminosity up by `factor` (Monaco `Color.lighten`).
    pub fn lighten(&self, factor: f64) -> Color {
        let hsla = self.hsla();
        Color::from_hsla(Hsla::new(hsla.h, hsla.s, hsla.l + hsla.l * factor, hsla.a))
    }

    /// Scales HSL luminosity down by `factor` (Monaco `Color.darken`).
    pub fn darken(&self, factor: f64) -> Color {
        let hsla = self.hsla();
        Color::from_hsla(Hsla::new(hsla.h, hsla.s, hsla.l - hsla.l * factor, hsla.a))
    }

    /// Multiplies alpha by `factor` (Monaco `Color.transparent`).
    pub fn transparent(&self, factor: f64) -> Color {
        let Rgba { r, g, b, a } = self.rgba;
        Color::new(Rgba::new_clamped(r.into(), g.into(), b.into(), a * factor))
    }

    /// Whether alpha is 0 (Monaco `Color.isTransparent`).
    pub fn is_transparent(&self) -> bool {
        self.rgba.a == 0.0
    }

    /// Whether alpha is 1 (Monaco `Color.isOpaque`).
    pub fn is_opaque(&self) -> bool {
        self.rgba.a == 1.0
    }

    /// Inverts the RGB channels, keeping alpha (Monaco `Color.opposite`).
    pub fn opposite(&self) -> Color {
        let Rgba { r, g, b, a } = self.rgba;
        Color::new(Rgba::new_clamped(
            f64::from(255 - r),
            f64::from(255 - g),
            f64::from(255 - b),
            a,
        ))
    }

    /// Composites this color over `c` (Monaco `Color.blend`).
    pub fn blend(&self, c: &Color) -> Color {
        let rgba = c.rgba;
        let this_a = self.rgba.a;
        let color_a = rgba.a;

        let a = this_a + color_a * (1.0 - this_a);
        if a < 1e-6 {
            return Color::TRANSPARENT;
        }

        let channel = |this: u8, other: u8| {
            f64::from(this) * this_a / a + f64::from(other) * color_a * (1.0 - this_a) / a
        };
        Color::new(Rgba::new_clamped(
            channel(self.rgba.r, rgba.r),
            channel(self.rgba.g, rgba.g),
            channel(self.rgba.b, rgba.b),
            a,
        ))
    }

    /// Linearly interpolates towards `color`; `factor` is clamped to
    /// `[0, 1]` (Monaco `Color.mix`, whose default factor is 0.5).
    pub fn mix(&self, color: &Color, factor: f64) -> Color {
        let normalize = js_min(js_max(factor, 0.0), 1.0);
        let this = self.rgba;
        let other = color.rgba;
        let lerp = |a: f64, b: f64| a + (b - a) * normalize;
        Color::new(Rgba::new_clamped(
            lerp(this.r.into(), other.r.into()),
            lerp(this.g.into(), other.g.into()),
            lerp(this.b.into(), other.b.into()),
            lerp(this.a, other.a),
        ))
    }

    /// The opaque color that looks like this one drawn over the opaque
    /// `opaque_background` (Monaco `Color.makeOpaque`).
    pub fn make_opaque(&self, opaque_background: &Color) -> Color {
        if self.is_opaque() || opaque_background.rgba.a != 1.0 {
            return *self;
        }
        let Rgba { r, g, b, a } = self.rgba;
        let bg = opaque_background.rgba;
        let channel = |bg: u8, fg: u8| f64::from(bg) - a * (f64::from(bg) - f64::from(fg));
        Color::new(Rgba::new_clamped(
            channel(bg.r, r),
            channel(bg.g, g),
            channel(bg.b, b),
            1.0,
        ))
    }

    /// Flattens this color onto `backgrounds`, the last one being the
    /// bottom-most (Monaco `Color.flatten`). Returns `self` if
    /// `backgrounds` is empty.
    pub fn flatten(&self, backgrounds: &[Color]) -> Color {
        let Some((last, rest)) = backgrounds.split_last() else {
            return *self;
        };
        let background = rest.iter().rev().fold(*last, |accumulator, color| {
            Self::flatten_one(color, &accumulator)
        });
        Self::flatten_one(self, &background)
    }

    /// Monaco `Color._flatten`.
    fn flatten_one(foreground: &Color, background: &Color) -> Color {
        let fg = foreground.rgba;
        let bg = background.rgba;
        let background_alpha = 1.0 - fg.a;
        let channel = |bg: u8, fg_c: u8| background_alpha * f64::from(bg) + fg.a * f64::from(fg_c);
        Color::new(Rgba::new_clamped(
            channel(bg.r, fg.r),
            channel(bg.g, fg.g),
            channel(bg.b, fg.b),
            1.0,
        ))
    }

    /// Packs the color as `0xRRGGBBAA` (Monaco `Color.toNumber32Bit`).
    pub fn to_number_32bit(&self) -> u32 {
        let Rgba { r, g, b, a } = self.rgba;
        (u32::from(r) << 24)
            | (u32::from(g) << 16)
            | (u32::from(b) << 8)
            | (js_trunc(a * 255.0) as u32)
    }
}

impl PartialEq for Color {
    fn eq(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

impl From<Rgba> for Color {
    fn from(rgba: Rgba) -> Self {
        Self::new(rgba)
    }
}

impl From<Hsla> for Color {
    fn from(hsla: Hsla) -> Self {
        Self::from_hsla(hsla)
    }
}

impl From<Hsva> for Color {
    fn from(hsva: Hsva) -> Self {
        Self::from_hsva(hsva)
    }
}

/// Formats with [`Color::format`] (Monaco `Color.toString`).
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&Color::format(self))
    }
}

/// CSS color formatting and parsing (Monaco `Color.Format.CSS`).
impl Color {
    /// `rgb(r, g, b)` when opaque, otherwise [`Color::format_rgba`] (Monaco
    /// `Color.Format.CSS.formatRGB`).
    pub fn format_rgb(color: &Color) -> String {
        let Rgba { r, g, b, a } = color.rgba();
        if a == 1.0 {
            return format!("rgb({r}, {g}, {b})");
        }
        Self::format_rgba(color)
    }

    /// `rgba(r, g, b, a)` with alpha to at most 2 decimals (Monaco
    /// `Color.Format.CSS.formatRGBA`).
    pub fn format_rgba(color: &Color) -> String {
        let Rgba { r, g, b, a } = color.rgba();
        // `+(a).toFixed(2)`: round, then print as a number (no trailing 0s).
        let a: f64 = js_to_fixed(a, 2).parse().unwrap_or(a);
        format!("rgba({r}, {g}, {b}, {a})")
    }

    /// `hsl(h, s%, l%)` when opaque, otherwise [`Color::format_hsla`] (Monaco
    /// `Color.Format.CSS.formatHSL`).
    pub fn format_hsl(color: &Color) -> String {
        let hsla = color.hsla();
        if hsla.a == 1.0 {
            return format!(
                "hsl({}, {}%, {}%)",
                hsla.h,
                js_round(hsla.s * 100.0),
                js_round(hsla.l * 100.0)
            );
        }
        Self::format_hsla(color)
    }

    /// `hsla(h, s%, l%, a)` with alpha to exactly 2 decimals (Monaco
    /// `Color.Format.CSS.formatHSLA`).
    pub fn format_hsla(color: &Color) -> String {
        let hsla = color.hsla();
        format!(
            "hsla({}, {}%, {}%, {})",
            hsla.h,
            js_round(hsla.s * 100.0),
            js_round(hsla.l * 100.0),
            js_to_fixed(hsla.a, 2)
        )
    }

    /// `#rrggbb`, ignoring alpha (Monaco `Color.Format.CSS.formatHex`).
    pub fn format_hex(color: &Color) -> String {
        let Rgba { r, g, b, .. } = color.rgba();
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// `#rrggbbaa`, or `#rrggbb` for an opaque color when `compact` (Monaco
    /// `Color.Format.CSS.formatHexA`).
    pub fn format_hex_a(color: &Color, compact: bool) -> String {
        let Rgba { r, g, b, a } = color.rgba();
        if compact && a == 1.0 {
            return Self::format_hex(color);
        }
        let a = js_round(a * 255.0) as u8;
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }

    /// Hex when opaque, otherwise `rgba(...)` (Monaco
    /// `Color.Format.CSS.format`).
    pub fn format(color: &Color) -> String {
        if color.is_opaque() {
            return Self::format_hex(color);
        }
        Self::format_rgba(color)
    }

    /// Parses `transparent`, hex, `rgb(...)`, `rgba(...)` and the CSS named
    /// colors (Monaco `Color.Format.CSS.parse`). A malformed `rgb(`/`rgba(`
    /// value returns `None` where Monaco throws.
    pub fn parse(css: &str) -> Option<Color> {
        if css == "transparent" {
            return Some(Self::TRANSPARENT);
        }
        if css.starts_with('#') {
            return Self::parse_hex(css);
        }
        let group = |caps: &Captures<'_>, name: &str| -> f64 {
            caps.name(name)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0.0)
        };
        if css.starts_with("rgba(") {
            let caps = PARSE_RGBA_RE.captures(css)?;
            let rgba = Rgba::new_clamped(
                group(&caps, "r"),
                group(&caps, "g"),
                group(&caps, "b"),
                group(&caps, "a"),
            );
            return Some(Color::new(rgba));
        }
        if css.starts_with("rgb(") {
            let caps = PARSE_RGB_RE.captures(css)?;
            let rgba =
                Rgba::new_clamped(group(&caps, "r"), group(&caps, "g"), group(&caps, "b"), 1.0);
            return Some(Color::new(rgba));
        }
        Self::parse_named_keyword(css)
    }

    /// Monaco `parseNamedKeyword`: the CSS named colors.
    fn parse_named_keyword(css: &str) -> Option<Color> {
        let (r, g, b) = match css {
            "aliceblue" => (240, 248, 255),
            "antiquewhite" => (250, 235, 215),
            "aqua" => (0, 255, 255),
            "aquamarine" => (127, 255, 212),
            "azure" => (240, 255, 255),
            "beige" => (245, 245, 220),
            "bisque" => (255, 228, 196),
            "black" => (0, 0, 0),
            "blanchedalmond" => (255, 235, 205),
            "blue" => (0, 0, 255),
            "blueviolet" => (138, 43, 226),
            "brown" => (165, 42, 42),
            "burlywood" => (222, 184, 135),
            "cadetblue" => (95, 158, 160),
            "chartreuse" => (127, 255, 0),
            "chocolate" => (210, 105, 30),
            "coral" => (255, 127, 80),
            "cornflowerblue" => (100, 149, 237),
            "cornsilk" => (255, 248, 220),
            "crimson" => (220, 20, 60),
            "cyan" => (0, 255, 255),
            "darkblue" => (0, 0, 139),
            "darkcyan" => (0, 139, 139),
            "darkgoldenrod" => (184, 134, 11),
            "darkgray" => (169, 169, 169),
            "darkgreen" => (0, 100, 0),
            "darkgrey" => (169, 169, 169),
            "darkkhaki" => (189, 183, 107),
            "darkmagenta" => (139, 0, 139),
            "darkolivegreen" => (85, 107, 47),
            "darkorange" => (255, 140, 0),
            "darkorchid" => (153, 50, 204),
            "darkred" => (139, 0, 0),
            "darksalmon" => (233, 150, 122),
            "darkseagreen" => (143, 188, 143),
            "darkslateblue" => (72, 61, 139),
            "darkslategray" => (47, 79, 79),
            "darkslategrey" => (47, 79, 79),
            "darkturquoise" => (0, 206, 209),
            "darkviolet" => (148, 0, 211),
            "deeppink" => (255, 20, 147),
            "deepskyblue" => (0, 191, 255),
            "dimgray" => (105, 105, 105),
            "dimgrey" => (105, 105, 105),
            "dodgerblue" => (30, 144, 255),
            "firebrick" => (178, 34, 34),
            "floralwhite" => (255, 250, 240),
            "forestgreen" => (34, 139, 34),
            "fuchsia" => (255, 0, 255),
            "gainsboro" => (220, 220, 220),
            "ghostwhite" => (248, 248, 255),
            "gold" => (255, 215, 0),
            "goldenrod" => (218, 165, 32),
            "gray" => (128, 128, 128),
            "green" => (0, 128, 0),
            "greenyellow" => (173, 255, 47),
            "grey" => (128, 128, 128),
            "honeydew" => (240, 255, 240),
            "hotpink" => (255, 105, 180),
            "indianred" => (205, 92, 92),
            "indigo" => (75, 0, 130),
            "ivory" => (255, 255, 240),
            "khaki" => (240, 230, 140),
            "lavender" => (230, 230, 250),
            "lavenderblush" => (255, 240, 245),
            "lawngreen" => (124, 252, 0),
            "lemonchiffon" => (255, 250, 205),
            "lightblue" => (173, 216, 230),
            "lightcoral" => (240, 128, 128),
            "lightcyan" => (224, 255, 255),
            "lightgoldenrodyellow" => (250, 250, 210),
            "lightgray" => (211, 211, 211),
            "lightgreen" => (144, 238, 144),
            "lightgrey" => (211, 211, 211),
            "lightpink" => (255, 182, 193),
            "lightsalmon" => (255, 160, 122),
            "lightseagreen" => (32, 178, 170),
            "lightskyblue" => (135, 206, 250),
            "lightslategray" => (119, 136, 153),
            "lightslategrey" => (119, 136, 153),
            "lightsteelblue" => (176, 196, 222),
            "lightyellow" => (255, 255, 224),
            "lime" => (0, 255, 0),
            "limegreen" => (50, 205, 50),
            "linen" => (250, 240, 230),
            "magenta" => (255, 0, 255),
            "maroon" => (128, 0, 0),
            "mediumaquamarine" => (102, 205, 170),
            "mediumblue" => (0, 0, 205),
            "mediumorchid" => (186, 85, 211),
            "mediumpurple" => (147, 112, 219),
            "mediumseagreen" => (60, 179, 113),
            "mediumslateblue" => (123, 104, 238),
            "mediumspringgreen" => (0, 250, 154),
            "mediumturquoise" => (72, 209, 204),
            "mediumvioletred" => (199, 21, 133),
            "midnightblue" => (25, 25, 112),
            "mintcream" => (245, 255, 250),
            "mistyrose" => (255, 228, 225),
            "moccasin" => (255, 228, 181),
            "navajowhite" => (255, 222, 173),
            "navy" => (0, 0, 128),
            "oldlace" => (253, 245, 230),
            "olive" => (128, 128, 0),
            "olivedrab" => (107, 142, 35),
            "orange" => (255, 165, 0),
            "orangered" => (255, 69, 0),
            "orchid" => (218, 112, 214),
            "palegoldenrod" => (238, 232, 170),
            "palegreen" => (152, 251, 152),
            "paleturquoise" => (175, 238, 238),
            "palevioletred" => (219, 112, 147),
            "papayawhip" => (255, 239, 213),
            "peachpuff" => (255, 218, 185),
            "peru" => (205, 133, 63),
            "pink" => (255, 192, 203),
            "plum" => (221, 160, 221),
            "powderblue" => (176, 224, 230),
            "purple" => (128, 0, 128),
            "rebeccapurple" => (102, 51, 153),
            "red" => (255, 0, 0),
            "rosybrown" => (188, 143, 143),
            "royalblue" => (65, 105, 225),
            "saddlebrown" => (139, 69, 19),
            "salmon" => (250, 128, 114),
            "sandybrown" => (244, 164, 96),
            "seagreen" => (46, 139, 87),
            "seashell" => (255, 245, 238),
            "sienna" => (160, 82, 45),
            "silver" => (192, 192, 192),
            "skyblue" => (135, 206, 235),
            "slateblue" => (106, 90, 205),
            "slategray" => (112, 128, 144),
            "slategrey" => (112, 128, 144),
            "snow" => (255, 250, 250),
            "springgreen" => (0, 255, 127),
            "steelblue" => (70, 130, 180),
            "tan" => (210, 180, 140),
            "teal" => (0, 128, 128),
            "thistle" => (216, 191, 216),
            "tomato" => (255, 99, 71),
            "turquoise" => (64, 224, 208),
            "violet" => (238, 130, 238),
            "wheat" => (245, 222, 179),
            "white" => (255, 255, 255),
            "whitesmoke" => (245, 245, 245),
            "yellow" => (255, 255, 0),
            "yellowgreen" => (154, 205, 50),
            _ => return None,
        };
        Some(Color::new(Rgba { r, g, b, a: 1.0 }))
    }

    /// Parses `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA` (Monaco
    /// `Color.Format.CSS.parseHex`). Non-hex digits read as 0, as in Monaco.
    pub fn parse_hex(hex: &str) -> Option<Color> {
        // Monaco measures and indexes UTF-16 code units.
        let units: Vec<u16> = hex.encode_utf16().collect();
        if units.first() != Some(&u16::from(b'#')) {
            return None;
        }
        let d = |i: usize| Self::parse_hex_digit(units[i]);
        let rgba = match units.len() {
            7 => Rgba::new_clamped(
                16.0 * d(1) + d(2),
                16.0 * d(3) + d(4),
                16.0 * d(5) + d(6),
                1.0,
            ),
            9 => Rgba::new_clamped(
                16.0 * d(1) + d(2),
                16.0 * d(3) + d(4),
                16.0 * d(5) + d(6),
                (16.0 * d(7) + d(8)) / 255.0,
            ),
            4 => {
                let (r, g, b) = (d(1), d(2), d(3));
                Rgba::new_clamped(16.0 * r + r, 16.0 * g + g, 16.0 * b + b, 1.0)
            }
            5 => {
                let (r, g, b, a) = (d(1), d(2), d(3), d(4));
                Rgba::new_clamped(
                    16.0 * r + r,
                    16.0 * g + g,
                    16.0 * b + b,
                    (16.0 * a + a) / 255.0,
                )
            }
            _ => return None,
        };
        Some(Color::new(rgba))
    }

    /// Monaco `_parseHexDigit`.
    fn parse_hex_digit(unit: u16) -> f64 {
        char::from_u32(unit.into())
            .and_then(|c| c.to_digit(16))
            .map_or(0.0, f64::from)
    }
}

static PARSE_RGBA_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"rgba\((?<r>(?:\+|-)?[0-9]+), *(?<g>(?:\+|-)?[0-9]+), *(?<b>(?:\+|-)?[0-9]+), *(?<a>(?:\+|-)?[0-9]+(\.[0-9]+)?)\)",
    )
    .expect("valid rgba regex")
});

static PARSE_RGB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"rgb\((?<r>(?:\+|-)?[0-9]+), *(?<g>(?:\+|-)?[0-9]+), *(?<b>(?:\+|-)?[0-9]+)\)")
        .expect("valid rgb regex")
});

/// A color with components in `[0, 1]` (Monaco `IColor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorF {
    /// Red, in `[0, 1]`.
    pub red: f64,
    /// Green, in `[0, 1]`.
    pub green: f64,
    /// Blue, in `[0, 1]`.
    pub blue: f64,
    /// Alpha, in `[0, 1]`.
    pub alpha: f64,
}

/// JavaScript's `\s` set, for use inside a character class.
const JS_SPACE: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// Compiles a JavaScript (non-unicode) regex source: `\s` becomes JS
/// whitespace, `\d` ASCII digits and `\b` an ASCII word boundary.
fn js_regex(source: &str) -> Regex {
    let mut out = String::with_capacity(source.len() * 2);
    let mut in_class = false;
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('s') if in_class => out.push_str(JS_SPACE),
                Some('s') => {
                    out.push('[');
                    out.push_str(JS_SPACE);
                    out.push(']');
                }
                Some('d') if in_class => out.push_str("0-9"),
                Some('d') => out.push_str("[0-9]"),
                Some('b') if !in_class => out.push_str(r"(?-u:\b)"),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            '[' if !in_class => {
                in_class = true;
                out.push(c);
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    Regex::new(&out).expect("valid color regex")
}

/// Monaco's `initialValidationRegex`. The hex alternatives `^(#)...` and
/// `(?<=['"\s])(#)...` are merged into one that consumes the preceding
/// character; under the `m` flag `^` only adds start-of-text, since every JS
/// line terminator is also `\s`. Groups: 1 scheme, 2 params, 3 `#`, 4 digits.
static INITIAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    js_regex(
        r#"\b(rgb|rgba|hsl|hsla)(\([0-9\s,.\%\/]*\))|(?:^|['"\s])(#)([A-Fa-f0-9]{3}|[A-Fa-f0-9]{4}|[A-Fa-f0-9]{6}|[A-Fa-f0-9]{8})\b"#,
    )
});

// The parameter strings contain exactly one `(` (first) and one `)` (last),
// so Monaco's `m` flag on these `^...$` patterns never matters.
static RGB_PARAMS_RE: LazyLock<Regex> = LazyLock::new(|| {
    js_regex(
        r"^\(\s*(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\s*[\s,]\s*(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\s*[\s,]\s*(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\s*\)$",
    )
});

static RGBA_PARAMS_RE: LazyLock<Regex> = LazyLock::new(|| {
    js_regex(
        r"^\(\s*(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\s*[\s,]\s*(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\s*[\s,]\s*(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\s*(?:[\s,]|[\s]*\/)\s*(0[.][0-9]+|[.][0-9]+|[01][.]|[01])\s*\)$",
    )
});

static HSL_PARAMS_RE: LazyLock<Regex> = LazyLock::new(|| {
    js_regex(
        r"^\(\s*((?:360(?:\.0+)?|(?:36[0]|3[0-5][0-9]|[12][0-9][0-9]|[1-9]?[0-9])(?:\.\d+)?))\s*[\s,]\s*(100(?:\.0+)?|\d{1,2}[.]\d*|\d{1,2})%\s*[\s,]\s*(100(?:\.0+)?|\d{1,2}[.]\d*|\d{1,2})%\s*\)$",
    )
});

static HSLA_PARAMS_RE: LazyLock<Regex> = LazyLock::new(|| {
    js_regex(
        r"^\(\s*((?:360(?:\.0+)?|(?:36[0]|3[0-5][0-9]|[12][0-9][0-9]|[1-9]?[0-9])(?:\.\d+)?))\s*[\s,]\s*(100(?:\.0+)?|\d{1,2}[.]\d*|\d{1,2})%\s*[\s,]\s*(100(?:\.0+)?|\d{1,2}[.]\d*|\d{1,2})%\s*(?:[\s,]|[\s]*\/)\s*(0[.][0-9]+|[.][0-9]+|[01][.]0*|[01])\s*\)$",
    )
});

/// Monaco `_parseCaptureGroups`: the numeric capture groups, in order. (The
/// whole match, Monaco's first value, never parses as a number.)
fn parse_capture_groups(caps: &Captures<'_>) -> Vec<f64> {
    caps.iter()
        .skip(1)
        .flatten()
        .filter_map(|m| m.as_str().parse::<f64>().ok())
        .collect()
}

/// Monaco `_toIColor`.
fn to_color_f(r: f64, g: f64, b: f64, a: f64) -> ColorF {
    ColorF {
        red: r / 255.0,
        green: g / 255.0,
        blue: b / 255.0,
        alpha: a,
    }
}

/// Finds the hex, `rgb()`, `rgba()`, `hsl()` and `hsla()` colors in `text`,
/// in document order, as byte ranges into `text` with their colors (Monaco
/// `computeDefaultDocumentColors`).
pub fn compute_default_document_colors(text: &str) -> Vec<(Range<usize>, ColorF)> {
    let mut result = Vec::new();
    for caps in INITIAL_RE.captures_iter(text) {
        let (scheme, params) = match (caps.get(1), caps.get(3)) {
            (Some(scheme), _) => (scheme, &caps[2]),
            (None, Some(hash)) => (hash, &caps[4]),
            (None, None) => continue,
        };
        let range = scheme.start()..caps.get(0).map_or(scheme.end(), |m| m.end());
        let rgb = |re: &Regex, alpha: bool| {
            let p = parse_capture_groups(&re.captures(params)?);
            Some(to_color_f(p[0], p[1], p[2], if alpha { p[3] } else { 1.0 }))
        };
        let hsl = |re: &Regex, alpha: bool| {
            let p = parse_capture_groups(&re.captures(params)?);
            let a = if alpha { p[3] } else { 1.0 };
            let Rgba { r, g, b, a } =
                Color::from_hsla(Hsla::new(p[0], p[1] / 100.0, p[2] / 100.0, a)).rgba();
            Some(to_color_f(r.into(), g.into(), b.into(), a))
        };
        let color = match scheme.as_str() {
            "rgb" => rgb(&RGB_PARAMS_RE, false),
            "rgba" => rgb(&RGBA_PARAMS_RE, true),
            "hsl" => hsl(&HSL_PARAMS_RE, false),
            "hsla" => hsl(&HSLA_PARAMS_RE, true),
            _ => Color::parse_hex(&format!("#{params}")).map(|c| {
                let Rgba { r, g, b, a } = c.rgba();
                to_color_f(r.into(), g.into(), b.into(), a)
            }),
        };
        if let Some(color) = color {
            result.push((range, color));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(r: f64, g: f64, b: f64, a: f64) -> Rgba {
        Rgba::new_clamped(r, g, b, a)
    }

    // ---- color.test.ts ----

    #[test]
    fn is_lighter_color() {
        let color1 = Color::from_hsla(Hsla::new(60.0, 1.0, 0.5, 1.0));
        let color2 = Color::from_hsla(Hsla::new(0.0, 0.0, 0.753, 1.0));
        assert!(color1.is_lighter_than(&color2));
        // Abyss theme
        assert!(Color::from_hex("#770811").is_lighter_than(&Color::from_hex("#000c18")));
    }

    #[test]
    fn get_lighter_and_darker_basics() {
        assert!(Color::WHITE.is_lighter());
        assert!(Color::BLACK.is_darker());
        assert!(!Color::WHITE.is_darker());
        assert!(Color::from_hex("#808080").is_lighter());
        assert!(Color::from_hex("#7f7f7f").is_darker());
        assert!(Color::BLACK.is_darker_than(&Color::WHITE));
    }

    #[test]
    fn relative_luminance_and_contrast() {
        assert_eq!(Color::WHITE.get_relative_luminance(), 1.0);
        assert_eq!(Color::BLACK.get_relative_luminance(), 0.0);
        assert_eq!(Color::RED.get_relative_luminance(), 0.2126);
        assert_eq!(Color::WHITE.get_contrast_ratio(&Color::BLACK), 21.0);
        assert_eq!(Color::BLACK.get_contrast_ratio(&Color::WHITE), 21.0);
    }

    #[test]
    fn opaque_and_transparent() {
        assert!(Color::WHITE.is_opaque());
        assert!(!Color::WHITE.is_transparent());
        assert!(Color::TRANSPARENT.is_transparent());
        assert_eq!(
            Color::WHITE.transparent(0.5).rgba(),
            rgba(255.0, 255.0, 255.0, 0.5)
        );
        assert_eq!(Color::WHITE.transparent(1.0 / 3.0).rgba().a, 0.333);
    }

    #[test]
    fn opposite() {
        let c = Color::new(rgba(10.0, 20.0, 30.0, 0.5)).opposite();
        assert_eq!(c.rgba(), rgba(245.0, 235.0, 225.0, 0.5));
    }

    #[test]
    fn blend() {
        assert_eq!(Color::RED.blend(&Color::BLUE).rgba(), Color::RED.rgba());
        let half = Color::new(rgba(255.0, 0.0, 0.0, 0.5));
        assert_eq!(
            half.blend(&Color::BLUE).rgba(),
            rgba(127.5, 0.0, 127.5, 1.0)
        );
        assert_eq!(
            half.blend(&Color::BLUE).rgba(),
            rgba(127.0, 0.0, 127.0, 1.0)
        );
        assert_eq!(
            Color::TRANSPARENT.blend(&Color::TRANSPARENT),
            Color::TRANSPARENT
        );
    }

    #[test]
    fn mix() {
        assert_eq!(
            Color::BLACK.mix(&Color::WHITE, 0.5).rgba(),
            rgba(127.0, 127.0, 127.0, 1.0)
        );
        assert_eq!(
            Color::BLACK.mix(&Color::WHITE, 2.0).rgba(),
            Color::WHITE.rgba()
        );
        assert_eq!(
            Color::BLACK.mix(&Color::WHITE, -1.0).rgba(),
            Color::BLACK.rgba()
        );
    }

    #[test]
    fn make_opaque() {
        let c = Color::new(rgba(255.0, 0.0, 0.0, 0.5));
        assert_eq!(
            c.make_opaque(&Color::WHITE).rgba(),
            rgba(255.0, 127.5, 127.5, 1.0)
        );
        // Only onto an opaque background.
        assert_eq!(c.make_opaque(&c).rgba(), c.rgba());
        assert_eq!(
            Color::RED.make_opaque(&Color::WHITE).rgba(),
            Color::RED.rgba()
        );
    }

    #[test]
    fn flatten() {
        let c = Color::new(rgba(255.0, 0.0, 0.0, 0.5));
        assert_eq!(
            c.flatten(&[Color::WHITE]).rgba(),
            rgba(255.0, 127.5, 127.5, 1.0)
        );
        let half_black = Color::new(rgba(0.0, 0.0, 0.0, 0.5));
        // half_black over white = 127.5 grey, then red over that.
        assert_eq!(
            c.flatten(&[half_black, Color::WHITE]).rgba(),
            rgba(127.5 + 63.75, 63.75, 63.75, 1.0)
        );
        assert_eq!(c.flatten(&[]).rgba(), c.rgba());
    }

    #[test]
    fn lighten_and_darken() {
        let c = Color::from_hsla(Hsla::new(0.0, 1.0, 0.4, 1.0));
        assert_eq!(c.lighten(0.5).hsla(), Hsla::new(0.0, 1.0, 0.6, 1.0));
        assert_eq!(c.darken(0.5).hsla(), Hsla::new(0.0, 1.0, 0.2, 1.0));
    }

    #[test]
    fn to_number_32bit() {
        assert_eq!(Color::WHITE.to_number_32bit(), 0xffff_ffff);
        assert_eq!(Color::from_hex("#01020380").to_number_32bit(), 0x0102_0380);
        assert_eq!(Color::TRANSPARENT.to_number_32bit(), 0);
    }

    #[test]
    fn rgba_constructor_clamps_and_rounds() {
        assert_eq!(
            Rgba::new_clamped(-10.0, 300.0, 12.9, 2.0),
            Rgba {
                r: 0,
                g: 255,
                b: 12,
                a: 1.0
            }
        );
        assert_eq!(Rgba::new_clamped(0.0, 0.0, 0.0, 0.12345).a, 0.123);
        assert_eq!(Rgba::new_clamped(0.0, 0.0, 0.0, 0.0005).a, 0.001);
        assert_eq!(Rgba::new_clamped(f64::NAN, 0.0, 0.0, 1.0).r, 0);
        assert_eq!(
            Hsla::new(400.0, -1.0, 0.5555, 0.5),
            Hsla {
                h: 360.0,
                s: 0.0,
                l: 0.556,
                a: 0.5
            }
        );
        assert_eq!(
            Hsva::new(120.9, 1.5, 0.3333, 0.5),
            Hsva {
                h: 120.0,
                s: 1.0,
                v: 0.333,
                a: 0.5
            }
        );
    }

    #[test]
    fn rgba_to_hsla() {
        let cases = [
            ((0.0, 0.0, 0.0, 0.0), (0.0, 0.0, 0.0, 0.0)),
            ((0.0, 0.0, 0.0, 1.0), (0.0, 0.0, 0.0, 1.0)),
            ((255.0, 255.0, 255.0, 1.0), (0.0, 0.0, 1.0, 1.0)),
            ((255.0, 0.0, 0.0, 1.0), (0.0, 1.0, 0.5, 1.0)),
            ((0.0, 255.0, 0.0, 1.0), (120.0, 1.0, 0.5, 1.0)),
            ((0.0, 0.0, 255.0, 1.0), (240.0, 1.0, 0.5, 1.0)),
            ((255.0, 255.0, 0.0, 1.0), (60.0, 1.0, 0.5, 1.0)),
            ((0.0, 255.0, 255.0, 1.0), (180.0, 1.0, 0.5, 1.0)),
            ((255.0, 0.0, 255.0, 1.0), (300.0, 1.0, 0.5, 1.0)),
            ((192.0, 192.0, 192.0, 1.0), (0.0, 0.0, 0.753, 1.0)),
            ((128.0, 128.0, 128.0, 1.0), (0.0, 0.0, 0.502, 1.0)),
            ((128.0, 0.0, 0.0, 1.0), (0.0, 1.0, 0.251, 1.0)),
            ((128.0, 128.0, 0.0, 1.0), (60.0, 1.0, 0.251, 1.0)),
            ((0.0, 128.0, 0.0, 1.0), (120.0, 1.0, 0.251, 1.0)),
            ((128.0, 0.0, 128.0, 1.0), (300.0, 1.0, 0.251, 1.0)),
            ((0.0, 128.0, 128.0, 1.0), (180.0, 1.0, 0.251, 1.0)),
            ((0.0, 0.0, 128.0, 1.0), (240.0, 1.0, 0.251, 1.0)),
        ];
        for ((r, g, b, a), (h, s, l, ha)) in cases {
            assert_eq!(
                Hsla::from_rgba(rgba(r, g, b, a)),
                Hsla::new(h, s, l, ha),
                "{r} {g} {b}"
            );
        }
    }

    #[test]
    fn hsla_to_rgba() {
        let cases = [
            ((0.0, 0.0, 0.0, 0.0), (0.0, 0.0, 0.0, 0.0)),
            ((0.0, 0.0, 1.0, 1.0), (255.0, 255.0, 255.0, 1.0)),
            ((0.0, 1.0, 0.5, 1.0), (255.0, 0.0, 0.0, 1.0)),
            ((120.0, 1.0, 0.5, 1.0), (0.0, 255.0, 0.0, 1.0)),
            ((240.0, 1.0, 0.5, 1.0), (0.0, 0.0, 255.0, 1.0)),
            ((60.0, 1.0, 0.5, 1.0), (255.0, 255.0, 0.0, 1.0)),
            ((180.0, 1.0, 0.5, 1.0), (0.0, 255.0, 255.0, 1.0)),
            ((300.0, 1.0, 0.5, 1.0), (255.0, 0.0, 255.0, 1.0)),
            ((0.0, 0.0, 0.753, 1.0), (192.0, 192.0, 192.0, 1.0)),
            ((0.0, 0.0, 0.502, 1.0), (128.0, 128.0, 128.0, 1.0)),
            ((0.0, 1.0, 0.251, 1.0), (128.0, 0.0, 0.0, 1.0)),
            ((60.0, 1.0, 0.251, 1.0), (128.0, 128.0, 0.0, 1.0)),
            ((120.0, 1.0, 0.251, 1.0), (0.0, 128.0, 0.0, 1.0)),
            ((300.0, 1.0, 0.251, 1.0), (128.0, 0.0, 128.0, 1.0)),
            ((180.0, 1.0, 0.251, 1.0), (0.0, 128.0, 128.0, 1.0)),
            ((240.0, 1.0, 0.251, 1.0), (0.0, 0.0, 128.0, 1.0)),
        ];
        for ((h, s, l, a), (r, g, b, ra)) in cases {
            assert_eq!(
                Hsla::new(h, s, l, a).to_rgba(),
                rgba(r, g, b, ra),
                "{h} {s} {l}"
            );
        }
    }

    #[test]
    fn rgba_to_hsva_and_back() {
        let cases = [
            ((0.0, 0.0, 0.0, 0.0), (0.0, 0.0, 0.0, 0.0)),
            ((0.0, 0.0, 0.0, 1.0), (0.0, 0.0, 0.0, 1.0)),
            ((255.0, 255.0, 255.0, 1.0), (0.0, 0.0, 1.0, 1.0)),
            ((255.0, 0.0, 0.0, 1.0), (0.0, 1.0, 1.0, 1.0)),
            ((0.0, 255.0, 0.0, 1.0), (120.0, 1.0, 1.0, 1.0)),
            ((0.0, 0.0, 255.0, 1.0), (240.0, 1.0, 1.0, 1.0)),
            ((255.0, 255.0, 0.0, 1.0), (60.0, 1.0, 1.0, 1.0)),
            ((0.0, 255.0, 255.0, 1.0), (180.0, 1.0, 1.0, 1.0)),
            ((255.0, 0.0, 255.0, 1.0), (300.0, 1.0, 1.0, 1.0)),
            ((192.0, 192.0, 192.0, 1.0), (0.0, 0.0, 0.753, 1.0)),
            ((128.0, 128.0, 128.0, 1.0), (0.0, 0.0, 0.502, 1.0)),
            ((128.0, 0.0, 0.0, 1.0), (0.0, 1.0, 0.502, 1.0)),
            ((128.0, 128.0, 0.0, 1.0), (60.0, 1.0, 0.502, 1.0)),
            ((0.0, 128.0, 0.0, 1.0), (120.0, 1.0, 0.502, 1.0)),
            ((128.0, 0.0, 128.0, 1.0), (300.0, 1.0, 0.502, 1.0)),
            ((0.0, 128.0, 128.0, 1.0), (180.0, 1.0, 0.502, 1.0)),
            ((0.0, 0.0, 128.0, 1.0), (240.0, 1.0, 0.502, 1.0)),
        ];
        for ((r, g, b, a), (h, s, v, ha)) in cases {
            assert_eq!(
                Hsva::from_rgba(rgba(r, g, b, a)),
                Hsva::new(h, s, v, ha),
                "{r} {g} {b}"
            );
            assert_eq!(
                Hsva::new(h, s, v, ha).to_rgba(),
                rgba(r, g, b, a),
                "{h} {s} {v}"
            );
        }
    }

    #[test]
    fn hsva_bug_36240() {
        let hsva = Hsva::from_rgba(rgba(92.0, 106.0, 196.0, 1.0));
        assert_eq!(hsva, Hsva::new(232.0, 0.531, 0.769, 1.0));
        assert_eq!(hsva.to_rgba(), rgba(92.0, 106.0, 196.0, 1.0));
    }

    #[test]
    fn color_keeps_hsla_and_hsva_it_was_built_from() {
        let hsla = Hsla::new(10.0, 0.123, 0.456, 1.0);
        let c = Color::from_hsla(hsla);
        assert_eq!(c.hsla(), hsla);
        assert_ne!(Hsla::from_rgba(c.rgba()), hsla);
        // So `equals` can tell it apart from the same RGB built from RGBA.
        let from_rgba = Color::new(c.rgba());
        assert_eq!(from_rgba.rgba(), c.rgba());
        assert!(!c.equals(&from_rgba));
        assert!(from_rgba.equals(&Color::new(c.rgba())));

        let hsva = Hsva::new(10.0, 0.123, 0.456, 1.0);
        assert_eq!(Color::from_hsva(hsva).hsva(), hsva);
    }

    #[test]
    fn parse_hex() {
        let hex = |s: &str| Color::parse_hex(s).map(|c| c.rgba());
        assert_eq!(hex(""), None);
        assert_eq!(hex("#"), None);
        assert_eq!(hex("#0102030"), None);
        assert_eq!(hex("000000"), None);
        assert_eq!(hex("#00"), None);
        assert_eq!(hex("#000000"), Some(rgba(0.0, 0.0, 0.0, 1.0)));
        assert_eq!(hex("#FFFFFF"), Some(rgba(255.0, 255.0, 255.0, 1.0)));
        assert_eq!(hex("#FF0000"), Some(rgba(255.0, 0.0, 0.0, 1.0)));
        assert_eq!(hex("#00FF00"), Some(rgba(0.0, 255.0, 0.0, 1.0)));
        assert_eq!(hex("#0000FF"), Some(rgba(0.0, 0.0, 255.0, 1.0)));
        assert_eq!(hex("#FFFF00"), Some(rgba(255.0, 255.0, 0.0, 1.0)));
        assert_eq!(hex("#00FFFF"), Some(rgba(0.0, 255.0, 255.0, 1.0)));
        assert_eq!(hex("#FF00FF"), Some(rgba(255.0, 0.0, 255.0, 1.0)));
        assert_eq!(hex("#C0C0C0"), Some(rgba(192.0, 192.0, 192.0, 1.0)));
        assert_eq!(hex("#808080"), Some(rgba(128.0, 128.0, 128.0, 1.0)));
        assert_eq!(hex("#800000"), Some(rgba(128.0, 0.0, 0.0, 1.0)));
        assert_eq!(hex("#808000"), Some(rgba(128.0, 128.0, 0.0, 1.0)));
        assert_eq!(hex("#008000"), Some(rgba(0.0, 128.0, 0.0, 1.0)));
        assert_eq!(hex("#800080"), Some(rgba(128.0, 0.0, 128.0, 1.0)));
        assert_eq!(hex("#008080"), Some(rgba(0.0, 128.0, 128.0, 1.0)));
        assert_eq!(hex("#000080"), Some(rgba(0.0, 0.0, 128.0, 1.0)));
        assert_eq!(hex("#010203"), Some(rgba(1.0, 2.0, 3.0, 1.0)));
        assert_eq!(hex("#040506"), Some(rgba(4.0, 5.0, 6.0, 1.0)));
        assert_eq!(hex("#070809"), Some(rgba(7.0, 8.0, 9.0, 1.0)));
        assert_eq!(hex("#0a0A0a"), Some(rgba(10.0, 10.0, 10.0, 1.0)));
        assert_eq!(hex("#0b0B0b"), Some(rgba(11.0, 11.0, 11.0, 1.0)));
        assert_eq!(hex("#0c0C0c"), Some(rgba(12.0, 12.0, 12.0, 1.0)));
        assert_eq!(hex("#0d0D0d"), Some(rgba(13.0, 13.0, 13.0, 1.0)));
        assert_eq!(hex("#0e0E0e"), Some(rgba(14.0, 14.0, 14.0, 1.0)));
        assert_eq!(hex("#0f0F0f"), Some(rgba(15.0, 15.0, 15.0, 1.0)));
        assert_eq!(hex("#a0A0a0"), Some(rgba(160.0, 160.0, 160.0, 1.0)));
        assert_eq!(hex("#CFA"), Some(rgba(204.0, 255.0, 170.0, 1.0)));
        assert_eq!(hex("#CFA8"), Some(rgba(204.0, 255.0, 170.0, 0.533)));
        assert_eq!(hex("#ff000080"), Some(rgba(255.0, 0.0, 0.0, 0.502)));
        // Non-hex digits read as 0; length counts UTF-16 code units.
        assert_eq!(hex("#zzz"), Some(rgba(0.0, 0.0, 0.0, 1.0)));
        assert_eq!(hex("#ff\u{1F600}"), Some(rgba(255.0, 255.0, 0.0, 0.0)));
    }

    #[test]
    fn from_hex_falls_back_to_red() {
        assert_eq!(Color::from_hex("nope"), Color::RED);
        assert_eq!(Color::from_hex("#00f"), Color::BLUE);
    }

    #[test]
    fn format_css() {
        let half_red = Color::new(rgba(255.0, 0.0, 0.0, 0.5));
        assert_eq!(Color::format_rgb(&Color::RED), "rgb(255, 0, 0)");
        assert_eq!(Color::format_rgb(&half_red), "rgba(255, 0, 0, 0.5)");
        assert_eq!(Color::format_rgba(&Color::RED), "rgba(255, 0, 0, 1)");
        assert_eq!(Color::format_rgba(&Color::TRANSPARENT), "rgba(0, 0, 0, 0)");
        // `toFixed(2)` rounds the exact tie 0.125 up; trailing zeros dropped.
        let eighth = Color::new(rgba(1.0, 2.0, 3.0, 0.125));
        assert_eq!(Color::format_rgba(&eighth), "rgba(1, 2, 3, 0.13)");
        assert_eq!(
            Color::format_rgba(&Color::new(rgba(1.0, 2.0, 3.0, 0.333))),
            "rgba(1, 2, 3, 0.33)"
        );
        assert_eq!(
            Color::format_rgba(&Color::new(rgba(1.0, 2.0, 3.0, 0.999))),
            "rgba(1, 2, 3, 1)"
        );

        assert_eq!(Color::format_hsl(&Color::RED), "hsl(0, 100%, 50%)");
        assert_eq!(
            Color::format_hsl(&Color::from_hex("#808080")),
            "hsl(0, 0%, 50%)"
        );
        assert_eq!(Color::format_hsl(&half_red), "hsla(0, 100%, 50%, 0.50)");
        assert_eq!(
            Color::format_hsla(&Color::GREEN),
            "hsla(120, 100%, 50%, 1.00)"
        );
        assert_eq!(Color::format_hsla(&eighth), "hsla(210, 50%, 1%, 0.13)");

        assert_eq!(Color::format_hex(&Color::from_hex("#0a0B0c")), "#0a0b0c");
        assert_eq!(Color::format_hex(&half_red), "#ff0000");
        assert_eq!(Color::format_hex_a(&Color::RED, false), "#ff0000ff");
        assert_eq!(Color::format_hex_a(&Color::RED, true), "#ff0000");
        assert_eq!(Color::format_hex_a(&half_red, true), "#ff000080");
        assert_eq!(Color::format_hex_a(&Color::TRANSPARENT, true), "#00000000");

        assert_eq!(Color::format(&Color::LIGHTGREY), "#d3d3d3");
        assert_eq!(Color::format(&half_red), "rgba(255, 0, 0, 0.5)");
        assert_eq!(Color::CYAN.to_string(), "#00ffff");
    }

    #[test]
    fn js_to_fixed_matches_javascript() {
        assert_eq!(js_to_fixed(0.125, 2), "0.13");
        assert_eq!(js_to_fixed(0.375, 2), "0.38");
        assert_eq!(js_to_fixed(1.005, 2), "1.00"); // 1.00499999...
        assert_eq!(js_to_fixed(0.995, 2), "0.99"); // 0.99499999...
        assert_eq!(js_to_fixed(0.9999, 2), "1.00");
        assert_eq!(js_to_fixed(9.995, 2), "9.99");
        assert_eq!(js_to_fixed(99.999, 2), "100.00");
        assert_eq!(js_to_fixed(0.5, 0), "1");
        assert_eq!(js_to_fixed(-1.5, 0), "-2");
        assert_eq!(js_to_fixed(-0.001, 2), "0.00");
        assert_eq!(js_to_fixed(0.0, 2), "0.00");
    }

    #[test]
    fn css_parse() {
        let p = |s: &str| Color::parse(s).map(|c| c.rgba());
        assert_eq!(Color::parse("transparent"), Some(Color::TRANSPARENT));
        assert_eq!(p("#ff0000"), Some(rgba(255.0, 0.0, 0.0, 1.0)));
        assert_eq!(p("#f008"), Some(rgba(255.0, 0.0, 0.0, 0.533)));
        assert_eq!(p("#ff"), None);
        assert_eq!(p("rgb(1, 2, 3)"), Some(rgba(1.0, 2.0, 3.0, 1.0)));
        assert_eq!(p("rgb(1,2,3)"), Some(rgba(1.0, 2.0, 3.0, 1.0)));
        assert_eq!(p("rgb(+1, -2, 300)"), Some(rgba(1.0, 0.0, 255.0, 1.0)));
        assert_eq!(p("rgba(1, 2, 3, 0.5)"), Some(rgba(1.0, 2.0, 3.0, 0.5)));
        assert_eq!(p("rgba(1, 2, 3, 1)"), Some(rgba(1.0, 2.0, 3.0, 1.0)));
        assert_eq!(
            p("rgba(1, 2, 3, 0.12345)"),
            Some(rgba(1.0, 2.0, 3.0, 0.123))
        );
        assert_eq!(p("rgba(1, 2, 3)"), None); // Monaco throws.
        assert_eq!(p("rgb(1 2 3)"), None); // Monaco throws.
        assert_eq!(p("red"), Some(rgba(255.0, 0.0, 0.0, 1.0)));
        assert_eq!(p("rebeccapurple"), Some(rgba(102.0, 51.0, 153.0, 1.0)));
        assert_eq!(p("yellowgreen"), Some(rgba(154.0, 205.0, 50.0, 1.0)));
        assert_eq!(p("Red"), None);
        assert_eq!(p("hsl(0, 100%, 50%)"), None);
    }

    // ---- defaultDocumentColorsComputer.test.ts ----

    fn colors(text: &str) -> Vec<(Range<usize>, ColorF)> {
        compute_default_document_colors(text)
    }

    fn rgb_of(info: &(Range<usize>, ColorF)) -> (f64, f64, f64, f64) {
        let c = info.1;
        (c.red, c.green, c.blue, c.alpha)
    }

    #[test]
    fn hex_colors_in_strings_should_be_detected() {
        let found = colors("const color = '#ff0000';");
        assert_eq!(found.len(), 1);
        assert_eq!(rgb_of(&found[0]), (1.0, 0.0, 0.0, 1.0));
        assert_eq!(found[0].0, 15..22);
    }

    #[test]
    fn hex_colors_in_double_quotes_should_be_detected() {
        let found = colors("const color = \"#00ff00\";");
        assert_eq!(found.len(), 1);
        assert_eq!(rgb_of(&found[0]), (0.0, 1.0, 0.0, 1.0));
    }

    #[test]
    fn multiple_hex_colors_in_array_should_be_detected() {
        let found = colors("const colors = ['#ff0000', '#00ff00', '#0000ff'];");
        assert_eq!(found.len(), 3);
        assert_eq!(rgb_of(&found[0]), (1.0, 0.0, 0.0, 1.0));
        assert_eq!(rgb_of(&found[1]), (0.0, 1.0, 0.0, 1.0));
        assert_eq!(rgb_of(&found[2]), (0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn existing_functionality_should_still_work() {
        for content in ["const color = ' #ff0000';", "#ff0000", "  #ff0000"] {
            assert_eq!(colors(content).len(), 1, "{content}");
        }
    }

    #[test]
    fn eight_digit_hex_colors_should_also_work() {
        let found = colors("const color = '#ff0000ff';");
        assert_eq!(found.len(), 1);
        assert_eq!(rgb_of(&found[0]), (1.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn hsl_percent_saturation_variants() {
        for content in [
            "const color = hsl(253, 100.00%, 47.10%);",
            "const color = hsl(253, 100%, 47.10%);",
            "const color = hsl(0, 83.60%, 47.80%);",
        ] {
            assert_eq!(colors(content).len(), 1, "{content}");
        }
    }

    #[test]
    fn hsl_with_decimal_hue_values_should_work() {
        for content in [
            "hsl(253.5, 100%, 50%)",
            "hsl(360.0, 50%, 50%)",
            "hsl(100.5, 50.5%, 50.5%)",
            "hsl(0.5, 50%, 50%)",
            "hsl(359.9, 100%, 50%)",
        ] {
            assert_eq!(
                colors(&format!("const color = {content};")).len(),
                1,
                "{content}"
            );
        }
    }

    #[test]
    fn hsla_with_decimal_values_should_work() {
        for content in [
            "hsla(253.5, 100%, 50%, 0.5)",
            "hsla(360.0, 50.5%, 50.5%, 1)",
            "hsla(0.5, 50%, 50%, 0.25)",
        ] {
            assert_eq!(
                colors(&format!("const color = {content};")).len(),
                1,
                "{content}"
            );
        }
    }

    #[test]
    fn hsl_with_space_separator_should_work() {
        for content in [
            "hsl(253 100% 50%)",
            "hsl(253.5 100% 50%)",
            "hsla(253 100% 50% / 0.5)",
            "hsla(253.5 100% 50% / 0.5)",
            "hsla(253 100% 50% / 1)",
        ] {
            assert_eq!(
                colors(&format!("const color = {content};")).len(),
                1,
                "{content}"
            );
        }
    }

    #[test]
    fn rgb_and_rgba_with_space_separated_syntax_should_work() {
        for content in [
            "rgb(255 0 0)",
            "rgb(128 128 128)",
            "rgba(255 0 0 / 0.5)",
            "rgba(128 128 128 / 0.8)",
            "rgba(255 0 0 / 1)",
            "rgb(255, 0, 0)",
            "rgba(255, 0, 0, 0.5)",
        ] {
            assert_eq!(
                colors(&format!("const color = {content};")).len(),
                1,
                "{content}"
            );
        }
    }

    // ---- additional behavior ----

    #[test]
    fn css_snippet() {
        let text = "color: #fff; background: rgba(255, 0, 0, 0.5); border: hsl(120, 100%, 50%)";
        let found = colors(text);
        assert_eq!(found.len(), 3);
        assert_eq!(&text[found[0].0.clone()], "#fff");
        assert_eq!(rgb_of(&found[0]), (1.0, 1.0, 1.0, 1.0));
        assert_eq!(&text[found[1].0.clone()], "rgba(255, 0, 0, 0.5)");
        assert_eq!(rgb_of(&found[1]), (1.0, 0.0, 0.0, 0.5));
        assert_eq!(&text[found[2].0.clone()], "hsl(120, 100%, 50%)");
        assert_eq!(rgb_of(&found[2]), (0.0, 1.0, 0.0, 1.0));
    }

    #[test]
    fn hex_lengths() {
        let found = colors("#abc #abcd #aabbcc #aabbccdd #ab #abcde #abcdefa #abcdefabc");
        let texts: Vec<_> = found.iter().map(|c| c.0.len()).collect();
        assert_eq!(texts, [4, 5, 7, 9]);
        let a = colors("#0f08")[0].1;
        assert_eq!((a.red, a.green, a.blue, a.alpha), (0.0, 1.0, 0.0, 0.533));
    }

    #[test]
    fn hex_needs_quote_whitespace_or_line_start_before_and_boundary_after() {
        assert!(colors("a#fff").is_empty());
        assert!(colors("(#fff)").is_empty());
        assert!(colors("#fffg").is_empty());
        assert!(colors("#fff_").is_empty());
        assert_eq!(colors("x\n#fff").len(), 1);
        assert_eq!(colors("x\r#fff").len(), 1);
        assert_eq!(colors("x\t#fff;").len(), 1);
        assert_eq!(colors("x\u{2028}#fff").len(), 1);
        assert_eq!(colors("x\u{FEFF}#fff").len(), 1);
        // Not JS whitespace.
        assert!(colors("x\u{85}#fff").is_empty());
        // ASCII word boundary: a non-ASCII letter ends the run.
        assert_eq!(colors("'#fffé").len(), 1);
        assert_eq!(colors("#fff#000").len(), 1);
        assert_eq!(colors("'#fff' '#000'").len(), 2);
    }

    #[test]
    fn rgb_needs_word_boundary_before() {
        assert!(colors("myrgb(1, 2, 3)").is_empty());
        assert!(colors("_rgb(1, 2, 3)").is_empty());
        assert_eq!(colors("-rgb(1, 2, 3)").len(), 1);
        // JS `\b` is ASCII-only, so a non-ASCII letter counts as a boundary.
        let found = colors("ärgb(1, 2, 3)");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 2..14);
    }

    #[test]
    fn invalid_functions_are_skipped() {
        for content in [
            "rgb(256, 0, 0)",
            "rgb(1, 2)",
            "rgb(1, 2, 3, 0.5)",
            "rgba(1, 2, 3)",
            "rgba(1, 2, 3, 2)",
            "rgba(1, 2, 3, 1.5)",
            "rgb(01, 2, 3)",
            "hsl(361, 50%, 50%)",
            "hsl(120, 101%, 50%)",
            "hsl(120, 50, 50)",
            "hsla(120, 50%, 50%)",
            "rgb (1, 2, 3)",
            "rgb(1, 2, 3",
            "RGB(1, 2, 3)",
        ] {
            assert!(colors(content).is_empty(), "{content}");
        }
    }

    #[test]
    fn rgba_alpha_is_not_rounded_but_hsla_alpha_is() {
        let c = colors("rgba(0, 0, 0, 0.12345)")[0].1;
        assert_eq!(c.alpha, 0.12345);
        let c = colors("rgba(0, 0, 0, .5)")[0].1;
        assert_eq!(c.alpha, 0.5);
        let c = colors("rgba(0, 0, 0, 1.)")[0].1;
        assert_eq!(c.alpha, 1.0);
        let c = colors("hsla(0, 0%, 0%, 0.12345)")[0].1;
        assert_eq!(c.alpha, 0.123);
    }

    #[test]
    fn hsl_converts_through_monaco_hsla() {
        // Hue is truncated (253.9 -> 253) as `new HSLA` does.
        let a = colors("hsl(253.9, 100%, 50%)")[0].1;
        let b = colors("hsl(253, 100%, 50%)")[0].1;
        assert_eq!(a, b);
        let rgba = Hsla::new(253.0, 1.0, 0.5, 1.0).to_rgba();
        assert_eq!(a.red, f64::from(rgba.r) / 255.0);
        assert_eq!(a.green, f64::from(rgba.g) / 255.0);
        assert_eq!(a.blue, f64::from(rgba.b) / 255.0);
    }

    #[test]
    fn byte_ranges_after_non_ascii_and_across_lines() {
        let text = "ü '#00f'\n  rgb(\n1, 2, 3)";
        let found = colors(text);
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].0.clone()], "#00f");
        assert_eq!(found[0].0, 4..8);
        assert_eq!(&text[found[1].0.clone()], "rgb(\n1, 2, 3)");
    }

    #[test]
    fn empty_text() {
        assert!(colors("").is_empty());
    }
}
