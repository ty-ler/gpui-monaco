//! The color registry: every theme color id the editor reads and its
//! default for each kind of theme, ported (MIT) from the `registerColor`
//! calls of Monaco's `editor/**` and of VS Code's
//! `platform/theme/common/colors/*.ts` (at the vendored commit
//! `53642551d756f7aefec1059e1b3961d3d2cc5adc`), plus the few workbench colors
//! the editor's defaults refer to.
//!
//! Defaults are expressions over other colors (`transparent(foreground,
//! 0.2)`), resolved lazily against a theme's own colors as Monaco's
//! `ColorRegistry.resolveDefaultColor` does, so a theme that sets a color
//! also moves every default derived from it.
//!
//! Workbench-only colors (activity bar, tabs, terminal, merge conflicts,
//! breadcrumbs, ...) are left out.

use std::collections::HashMap;
use std::sync::LazyLock;

use gpui::Hsla;

use crate::color::{Color, Rgba as ColorRgba};

/// A kind of theme, which picks the default of each color (Monaco
/// `ColorScheme`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ColorScheme {
    /// `vs-dark` and dark themes.
    Dark,
    /// `vs` and light themes.
    Light,
    /// `hc-black` and dark high contrast themes.
    HcDark,
    /// `hc-light` and light high contrast themes.
    HcLight,
}

impl ColorScheme {
    pub(crate) fn new(light: bool, high_contrast: bool) -> Self {
        match (light, high_contrast) {
            (false, false) => Self::Dark,
            (true, false) => Self::Light,
            (false, true) => Self::HcDark,
            (true, true) => Self::HcLight,
        }
    }

    pub(crate) fn is_light(self) -> bool {
        matches!(self, Self::Light | Self::HcLight)
    }

    pub(crate) fn is_high_contrast(self) -> bool {
        matches!(self, Self::HcDark | Self::HcLight)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A default color value (Monaco `ColorValue` and `ColorTransform`).
#[derive(Debug)]
pub(crate) enum V {
    /// No color (`null`).
    N,
    /// A literal `0xRRGGBBAA`.
    H(u32),
    /// Another color id.
    R(&'static str),
    /// `transparent(value, factor)`.
    T(&'static V, f32),
    /// `darken(value, factor)`.
    D(&'static V, f32),
    /// `lighten(value, factor)`.
    L(&'static V, f32),
    /// `oneOf(...values)`: the first that resolves.
    O(&'static [V]),
    /// `ifDefinedThenElse(id, then, else)`: `then` if the theme sets `id`.
    If(&'static str, &'static V, &'static V),
    /// `lessProminent(value, background, factor, transparency)`.
    LP(&'static V, &'static V, f32, f32),
}

use V::*;

fn to_color(color: Hsla) -> Color {
    let rgba = gpui::Rgba::from(color);
    let channel = |c: f32| (c.clamp(0., 1.) * 255.).round() as u8;
    Color::new(ColorRgba::new(
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        f64::from(rgba.a),
    ))
}

fn from_color(color: Color) -> Hsla {
    let ColorRgba { r, g, b, a } = color.rgba();
    gpui::Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: a as f32,
    }
    .into()
}

/// Monaco `Color.getLighterColor`.
fn lighter_color(of: Color, relative: Color, factor: f64) -> Color {
    if of.is_lighter_than(&relative) {
        return of;
    }
    let factor = if factor == 0. { 0.5 } else { factor };
    let lum1 = of.get_relative_luminance();
    let lum2 = relative.get_relative_luminance();
    of.lighten(factor * (lum2 - lum1) / lum2)
}

/// Monaco `Color.getDarkerColor`.
fn darker_color(of: Color, relative: Color, factor: f64) -> Color {
    if of.is_darker_than(&relative) {
        return of;
    }
    let factor = if factor == 0. { 0.5 } else { factor };
    let lum1 = of.get_relative_luminance();
    let lum2 = relative.get_relative_luminance();
    of.darken(factor * (lum1 - lum2) / lum1)
}

/// Resolves color ids against a theme's own colors, falling back to the
/// registry defaults for its [`ColorScheme`] (Monaco `IColorTheme.getColor`
/// with `useDefault`).
pub(crate) struct Resolver<'a> {
    scheme: ColorScheme,
    colors: &'a HashMap<String, Hsla>,
    cache: HashMap<String, Option<Hsla>>,
    depth: usize,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(scheme: ColorScheme, colors: &'a HashMap<String, Hsla>) -> Self {
        Self {
            scheme,
            colors,
            cache: HashMap::new(),
            depth: 0,
        }
    }

    pub(crate) fn scheme(&self) -> ColorScheme {
        self.scheme
    }

    /// Whether the theme sets `id` itself (Monaco `IColorTheme.defines`).
    pub(crate) fn defines(&self, id: &str) -> bool {
        self.colors.contains_key(id)
    }

    /// The theme's color `id`, else its default; `None` when both are null.
    pub(crate) fn get(&mut self, id: &str) -> Option<Hsla> {
        if let Some(color) = self.colors.get(id) {
            return Some(*color);
        }
        if let Some(color) = self.cache.get(id) {
            return *color;
        }
        // Registry defaults don't form cycles, but a bad one shouldn't hang.
        if self.depth > 32 {
            return None;
        }
        let color = match REGISTRY_BY_ID.get(id) {
            Some(defaults) => {
                self.depth += 1;
                let color = self.eval(&defaults[self.scheme.index()]);
                self.depth -= 1;
                color
            }
            None => None,
        };
        self.cache.insert(id.to_string(), color);
        color
    }

    /// [`Resolver::get`], transparent when null.
    pub(crate) fn color(&mut self, id: &str) -> Hsla {
        self.get(id).unwrap_or_else(gpui::transparent_black)
    }

    /// [`Resolver::get`], `None` also when transparent: for borders and
    /// outlines, which are only drawn when visible.
    pub(crate) fn visible(&mut self, id: &str) -> Option<Hsla> {
        self.get(id).filter(|color| color.a > 0.)
    }

    /// The first of `ids` that resolves.
    pub(crate) fn first(&mut self, ids: &[&str]) -> Option<Hsla> {
        ids.iter().find_map(|id| self.get(id))
    }

    /// [`Resolver::first`], transparent when none resolves.
    pub(crate) fn color_first(&mut self, ids: &[&str]) -> Hsla {
        self.first(ids).unwrap_or_else(gpui::transparent_black)
    }

    fn eval(&mut self, value: &V) -> Option<Hsla> {
        Some(match value {
            N => return None,
            H(rgba) => gpui::rgba(*rgba).into(),
            R(id) => return self.get(id),
            T(value, factor) => {
                let color = self.eval(value)?;
                Hsla {
                    a: color.a * factor,
                    ..color
                }
            }
            D(value, factor) => from_color(to_color(self.eval(value)?).darken(f64::from(*factor))),
            L(value, factor) => from_color(to_color(self.eval(value)?).lighten(f64::from(*factor))),
            O(values) => return values.iter().find_map(|value| self.eval(value)),
            If(id, then, otherwise) => {
                return if self.defines(id) {
                    self.eval(then)
                } else {
                    self.eval(otherwise)
                };
            }
            LP(value, background, factor, transparency) => {
                let from = self.eval(value)?;
                let Some(background) = self.eval(background) else {
                    return Some(Hsla {
                        a: from.a * factor * transparency,
                        ..from
                    });
                };
                let (from, background) = (to_color(from), to_color(background));
                let factor = f64::from(*factor);
                let color = if from.is_darker_than(&background) {
                    lighter_color(from, background, factor)
                } else {
                    darker_color(from, background, factor)
                };
                from_color(color.transparent(f64::from(*transparency)))
            }
        })
    }
}

static REGISTRY_BY_ID: LazyLock<HashMap<&'static str, &'static [V; 4]>> =
    LazyLock::new(|| REGISTRY.iter().map(|(id, v)| (*id, v)).collect());

/// Whether `id` is a registered color.
#[cfg(test)]
pub(crate) fn is_registered(id: &str) -> bool {
    REGISTRY_BY_ID.contains_key(id)
}

/// Color ids and their defaults for dark, light, hcDark and hcLight themes.
#[rustfmt::skip]
static REGISTRY: &[(&str, [V; 4])] = &[
    // The diff editor's gutter toolbar colors, which only VS Code's
    // workbench registers (the editor uses them with no fallback).
    ("editorGutter.itemBackground", [H(0x37373dff), H(0xd4d7e9ff), H(0xffffffff), H(0x000000ff)]),
    ("editorGutter.itemGlyphForeground", [R("editor.foreground"), R("editor.foreground"), H(0x000000ff), H(0xffffffff)]),
    // Workbench colors (`workbench/common/theme.ts`) that editor defaults
    // refer to.
    ("tab.inactiveBackground", [H(0x2d2d2dff), H(0xecececff), N, N]),
    ("sideBarSectionHeader.border", [R("contrastBorder"), R("contrastBorder"), R("contrastBorder"), R("contrastBorder")]),
    // The multi diff editor's compact and card variants' (`style.css`).
    ("sideBarSectionHeader.background", [T(&H(0x808080ff), 0.2), T(&H(0x808080ff), 0.2), N, N]),
    ("panel.border", [T(&H(0x808080ff), 0.35), T(&H(0x808080ff), 0.35), R("contrastBorder"), R("contrastBorder")]),
    // The git extension's decoration colors (its `package.json`).
    ("gitDecoration.addedResourceForeground", [H(0x81b88bff), H(0x587c0cff), H(0xa1e3adff), H(0x374e06ff)]),
    ("gitDecoration.deletedResourceForeground", [H(0xc74e39ff), H(0xad0707ff), H(0xc74e39ff), H(0xad0707ff)]),
    ("gitDecoration.modifiedResourceForeground", [H(0xe2c08dff), H(0x895503ff), H(0xe2c08dff), H(0x895503ff)]),
    ("banner.background", [R("list.activeSelectionBackground"), D(&R("list.activeSelectionBackground"), 0.3), R("list.activeSelectionBackground"), R("list.activeSelectionBackground")]),
    ("banner.foreground", [R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground")]),
    ("banner.iconForeground", [R("editorInfo.foreground"), R("editorInfo.foreground"), R("editorInfo.foreground"), R("editorInfo.foreground")]),
    // `platform/theme/common/colors/*.ts` and `editor/**`.
    ("foreground", [H(0xccccccff), H(0x616161ff), H(0xffffffff), H(0x292929ff)]),
    ("disabledForeground", [H(0xcccccc80), H(0x61616180), H(0xa5a5a5ff), H(0x7f7f7fff)]),
    ("errorForeground", [H(0xf48771ff), H(0xa1260dff), H(0xf48771ff), H(0xb5200dff)]),
    ("descriptionForeground", [T(&R("foreground"), 0.7), H(0x717171ff), T(&R("foreground"), 0.7), T(&R("foreground"), 0.7)]),
    ("icon.foreground", [H(0xc5c5c5ff), H(0x424242ff), H(0xffffffff), H(0x292929ff)]),
    ("focusBorder", [H(0x007fd4ff), H(0x0090f1ff), H(0xf38518ff), H(0x006bbdff)]),
    ("contrastBorder", [N, N, H(0x6fc3dfff), H(0x0f4a85ff)]),
    ("contrastActiveBorder", [N, N, R("focusBorder"), R("focusBorder")]),
    ("textLink.foreground", [H(0x3794ffff), H(0x006ab1ff), H(0x21a6ffff), H(0x0f4a85ff)]),
    ("textLink.activeForeground", [H(0x3794ffff), H(0x006ab1ff), H(0x21a6ffff), H(0x0f4a85ff)]),
    ("textCodeBlock.background", [H(0x0a0a0a66), H(0xdcdcdc66), H(0x000000ff), H(0xf2f2f2ff)]),
    ("textBlockQuote.background", [H(0x222222ff), H(0xf2f2f2ff), N, H(0xf2f2f2ff)]),
    ("textBlockQuote.border", [H(0x007acc80), H(0x0064c880), H(0xffffffff), H(0x292929ff)]),
    ("textSeparator.foreground", [H(0xffffff2e), H(0x0000002e), H(0x000000ff), H(0x292929ff)]),
    ("editor.background", [H(0x1e1e1eff), H(0xffffffff), H(0x000000ff), H(0xffffffff)]),
    ("editor.foreground", [H(0xbbbbbbff), H(0x333333ff), H(0xffffffff), R("foreground")]),
    ("editorStickyScroll.background", [R("editor.background"), R("editor.background"), R("editor.background"), R("editor.background")]),
    ("editorStickyScrollGutter.background", [R("editor.background"), R("editor.background"), R("editor.background"), R("editor.background")]),
    ("editorStickyScrollHover.background", [H(0x2a2d2eff), H(0xf0f0f0ff), N, H(0x0f4a851a)]),
    ("editorStickyScroll.border", [N, N, R("contrastBorder"), R("contrastBorder")]),
    ("editorStickyScroll.shadow", [R("scrollbar.shadow"), R("scrollbar.shadow"), R("scrollbar.shadow"), R("scrollbar.shadow")]),
    ("editorWidget.background", [H(0x252526ff), H(0xf3f3f3ff), H(0x0c141fff), H(0xffffffff)]),
    ("editorWidget.foreground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("editorWidget.border", [T(&R("editorWidget.foreground"), 0.2), T(&R("editorWidget.foreground"), 0.2), R("contrastBorder"), R("contrastBorder")]),
    ("editorWidget.resizeBorder", [N, N, N, N]),
    ("editorError.background", [N, N, N, N]),
    ("editorError.foreground", [H(0xf14c4cff), H(0xe51400ff), H(0xf48771ff), H(0xb5200dff)]),
    ("editorError.border", [N, N, H(0xe47777cc), H(0xb5200dff)]),
    ("editorWarning.background", [N, N, N, N]),
    ("editorWarning.foreground", [H(0xcca700ff), H(0xbf8803ff), H(0xffd370ff), H(0x895503ff)]),
    ("editorWarning.border", [N, N, H(0xffcc00cc), H(0xffcc00cc)]),
    ("editorInfo.background", [N, N, N, N]),
    ("editorInfo.foreground", [H(0x59a4f9ff), H(0x0063d3ff), H(0x59a4f9ff), H(0x0063d3ff)]),
    ("editorInfo.border", [N, N, H(0x59a4f9cc), H(0x292929ff)]),
    ("editorHint.foreground", [H(0xeeeeeeb3), H(0x6c6c6cff), N, N]),
    ("editorHint.border", [N, N, H(0xeeeeeecc), H(0x292929ff)]),
    ("editorLink.activeForeground", [H(0x4e94ceff), H(0x0000ffff), H(0x00ffffff), H(0x292929ff)]),
    ("editor.selectionBackground", [H(0x264f78ff), H(0xadd6ffff), H(0xf3f518ff), H(0x0f4a85ff)]),
    ("editor.selectionForeground", [N, N, H(0x000000ff), H(0xffffffff)]),
    ("editor.inactiveSelectionBackground", [T(&R("editor.selectionBackground"), 0.5), T(&R("editor.selectionBackground"), 0.5), T(&R("editor.selectionBackground"), 0.7), T(&R("editor.selectionBackground"), 0.5)]),
    ("editor.selectionHighlightBackground", [LP(&R("editor.selectionBackground"), &R("editor.background"), 0.3, 0.6), LP(&R("editor.selectionBackground"), &R("editor.background"), 0.3, 0.6), N, N]),
    ("editor.selectionHighlightBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editor.compositionBorder", [H(0xffffffff), H(0x000000ff), H(0xffffffff), H(0x000000ff)]),
    ("editor.findMatchBackground", [H(0x515c6aff), H(0xa8ac94ff), N, N]),
    ("editor.findMatchForeground", [N, N, N, N]),
    ("editor.findMatchHighlightBackground", [H(0xea5c0055), H(0xea5c0055), N, N]),
    ("editor.findMatchHighlightForeground", [N, N, N, N]),
    ("editor.findRangeHighlightBackground", [H(0x3a3d4166), H(0xb4b4b44d), N, N]),
    ("editor.findMatchBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editor.findMatchHighlightBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editor.findRangeHighlightBorder", [N, N, T(&R("contrastActiveBorder"), 0.4), T(&R("contrastActiveBorder"), 0.4)]),
    ("editor.hoverHighlightBackground", [H(0x264f7840), H(0xadd6ff26), H(0xadd6ff26), N]),
    ("editorHoverWidget.background", [R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background")]),
    ("editorHoverWidget.foreground", [R("editorWidget.foreground"), R("editorWidget.foreground"), R("editorWidget.foreground"), R("editorWidget.foreground")]),
    ("editorHoverWidget.border", [R("editorWidget.border"), R("editorWidget.border"), R("editorWidget.border"), R("editorWidget.border")]),
    ("editorHoverWidget.statusBarBackground", [L(&R("editorHoverWidget.background"), 0.2), D(&R("editorHoverWidget.background"), 0.05), R("editorWidget.background"), R("editorWidget.background")]),
    ("editorInlayHint.foreground", [H(0x969696ff), H(0x969696ff), H(0xffffffff), H(0x000000ff)]),
    ("editorInlayHint.background", [T(&R("badge.background"), 0.1), T(&R("badge.background"), 0.1), T(&H(0xffffffff), 0.1), T(&R("badge.background"), 0.1)]),
    ("editorInlayHint.typeForeground", [R("editorInlayHint.foreground"), R("editorInlayHint.foreground"), R("editorInlayHint.foreground"), R("editorInlayHint.foreground")]),
    ("editorInlayHint.typeBackground", [R("editorInlayHint.background"), R("editorInlayHint.background"), R("editorInlayHint.background"), R("editorInlayHint.background")]),
    ("editorInlayHint.parameterForeground", [R("editorInlayHint.foreground"), R("editorInlayHint.foreground"), R("editorInlayHint.foreground"), R("editorInlayHint.foreground")]),
    ("editorInlayHint.parameterBackground", [R("editorInlayHint.background"), R("editorInlayHint.background"), R("editorInlayHint.background"), R("editorInlayHint.background")]),
    ("editorLightBulb.foreground", [H(0xffcc00ff), H(0xddb100ff), H(0xffcc00ff), H(0x007accff)]),
    ("editorLightBulbAutoFix.foreground", [H(0x75beffff), H(0x007accff), H(0x75beffff), H(0x007accff)]),
    ("editorLightBulbAi.foreground", [R("editorLightBulb.foreground"), R("editorLightBulb.foreground"), R("editorLightBulb.foreground"), R("editorLightBulb.foreground")]),
    ("editor.snippetTabstopHighlightBackground", [H(0x7c7c7c4d), H(0x0a326433), H(0x7c7c7c4d), H(0x0a326433)]),
    ("editor.snippetTabstopHighlightBorder", [N, N, N, N]),
    ("editor.snippetFinalTabstopHighlightBackground", [N, N, N, N]),
    ("editor.snippetFinalTabstopHighlightBorder", [H(0x525252ff), H(0x0a326480), H(0x525252ff), H(0x292929ff)]),
    ("diffEditor.insertedTextBackground", [H(0x9ccc2c33), H(0x9ccc2c40), N, N]),
    ("diffEditor.removedTextBackground", [H(0xff000033), H(0xff000033), N, N]),
    ("diffEditor.insertedLineBackground", [H(0x9bb95533), H(0x9bb95533), N, N]),
    ("diffEditor.removedLineBackground", [H(0xff000033), H(0xff000033), N, N]),
    ("diffEditorGutter.insertedLineBackground", [N, N, N, N]),
    ("diffEditorGutter.removedLineBackground", [N, N, N, N]),
    ("diffEditorOverview.insertedForeground", [N, N, N, N]),
    ("diffEditorOverview.removedForeground", [N, N, N, N]),
    ("diffEditor.insertedTextBorder", [N, N, H(0x33ff2eff), H(0x374e06ff)]),
    ("diffEditor.removedTextBorder", [N, N, H(0xff008fff), H(0xad0707ff)]),
    ("diffEditor.border", [N, N, R("contrastBorder"), R("contrastBorder")]),
    ("diffEditor.diagonalFill", [H(0xcccccc33), H(0x22222233), N, N]),
    ("diffEditor.unchangedRegionBackground", [R("sideBar.background"), R("sideBar.background"), R("sideBar.background"), R("sideBar.background")]),
    ("diffEditor.unchangedRegionForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("diffEditor.unchangedCodeBackground", [H(0x74747429), H(0xb8b8b829), N, N]),
    ("widget.shadow", [T(&H(0x000000ff), 0.36), T(&H(0x000000ff), 0.16), N, N]),
    ("widget.border", [N, N, R("contrastBorder"), R("contrastBorder")]),
    ("toolbar.hoverBackground", [H(0x5a5d5e50), H(0xb8b8b850), N, N]),
    ("toolbar.hoverOutline", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("toolbar.activeBackground", [L(&R("toolbar.hoverBackground"), 0.1), D(&R("toolbar.hoverBackground"), 0.1), N, N]),
    ("editorOverviewRuler.findMatchForeground", [H(0xd186167e), H(0xd186167e), H(0xab5a00ff), H(0xab5a00ff)]),
    ("editorOverviewRuler.selectionHighlightForeground", [H(0xa0a0a0cc), H(0xa0a0a0cc), H(0xa0a0a0cc), H(0xa0a0a0cc)]),
    ("input.background", [H(0x3c3c3cff), H(0xffffffff), H(0x000000ff), H(0xffffffff)]),
    ("input.foreground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("input.border", [N, N, R("contrastBorder"), R("contrastBorder")]),
    ("inputOption.activeBorder", [H(0x007accff), H(0x007accff), R("contrastBorder"), R("contrastBorder")]),
    ("inputOption.hoverBackground", [H(0x5a5d5e80), H(0xb8b8b850), N, N]),
    ("inputOption.activeBackground", [T(&R("focusBorder"), 0.4), T(&R("focusBorder"), 0.2), H(0x00000000), H(0x00000000)]),
    ("inputOption.activeForeground", [H(0xffffffff), H(0x000000ff), R("foreground"), R("foreground")]),
    ("input.placeholderForeground", [T(&R("foreground"), 0.5), T(&R("foreground"), 0.5), T(&R("foreground"), 0.7), T(&R("foreground"), 0.7)]),
    ("inputValidation.infoBackground", [H(0x063b49ff), H(0xd6ecf2ff), H(0x000000ff), H(0xffffffff)]),
    ("inputValidation.infoForeground", [N, N, N, R("foreground")]),
    ("inputValidation.infoBorder", [H(0x007accff), H(0x007accff), R("contrastBorder"), R("contrastBorder")]),
    ("inputValidation.warningBackground", [H(0x352a05ff), H(0xf6f5d2ff), H(0x000000ff), H(0xffffffff)]),
    ("inputValidation.warningForeground", [N, N, N, R("foreground")]),
    ("inputValidation.warningBorder", [H(0xb89500ff), H(0xb89500ff), R("contrastBorder"), R("contrastBorder")]),
    ("inputValidation.errorBackground", [H(0x5a1d1dff), H(0xf2dedeff), H(0x000000ff), H(0xffffffff)]),
    ("inputValidation.errorForeground", [N, N, N, R("foreground")]),
    ("inputValidation.errorBorder", [H(0xbe1100ff), H(0xbe1100ff), R("contrastBorder"), R("contrastBorder")]),
    ("dropdown.background", [H(0x3c3c3cff), H(0xffffffff), H(0x000000ff), H(0xffffffff)]),
    ("dropdown.foreground", [H(0xf0f0f0ff), R("foreground"), H(0xffffffff), R("foreground")]),
    ("dropdown.border", [R("dropdown.background"), H(0xcececeff), R("contrastBorder"), R("contrastBorder")]),
    ("button.foreground", [H(0xffffffff), H(0xffffffff), H(0xffffffff), H(0xffffffff)]),
    ("button.separator", [T(&R("button.foreground"), 0.4), T(&R("button.foreground"), 0.4), R("contrastBorder"), R("contrastBorder")]),
    ("button.background", [H(0x0e639cff), H(0x007accff), H(0x000000ff), H(0x0f4a85ff)]),
    ("button.hoverBackground", [L(&R("button.background"), 0.2), D(&R("button.background"), 0.2), R("button.background"), R("button.background")]),
    ("button.border", [R("contrastBorder"), R("contrastBorder"), R("contrastBorder"), R("contrastBorder")]),
    ("button.secondaryForeground", [R("foreground"), R("foreground"), H(0xffffffff), R("foreground")]),
    ("button.secondaryBackground", [R("list.hoverBackground"), R("list.hoverBackground"), N, H(0xffffffff)]),
    ("button.secondaryBorder", [T(&R("foreground"), 0.15), T(&R("foreground"), 0.15), R("contrastBorder"), R("contrastBorder")]),
    ("button.secondaryHoverBackground", [L(&R("list.hoverBackground"), 0.2), L(&R("list.hoverBackground"), 0.2), N, N]),
    ("checkbox.background", [R("dropdown.background"), R("dropdown.background"), R("dropdown.background"), R("dropdown.background")]),
    ("checkbox.selectBackground", [R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background")]),
    ("checkbox.foreground", [R("dropdown.foreground"), R("dropdown.foreground"), R("dropdown.foreground"), R("dropdown.foreground")]),
    ("checkbox.border", [R("dropdown.border"), R("dropdown.border"), R("dropdown.border"), R("dropdown.border")]),
    ("checkbox.selectBorder", [R("icon.foreground"), R("icon.foreground"), R("icon.foreground"), R("icon.foreground")]),
    ("keybindingLabel.background", [H(0x8080802b), H(0xdddddd66), H(0x00000000), H(0x00000000)]),
    ("keybindingLabel.foreground", [H(0xccccccff), H(0x555555ff), H(0xffffffff), R("foreground")]),
    ("keybindingLabel.border", [H(0x33333399), H(0xcccccc66), H(0x6fc3dfff), R("contrastBorder")]),
    ("keybindingLabel.bottomBorder", [H(0x44444499), H(0xbbbbbb66), H(0x6fc3dfff), R("foreground")]),
    ("list.focusBackground", [N, N, N, N]),
    ("list.focusForeground", [N, N, N, N]),
    ("list.focusOutline", [R("focusBorder"), R("focusBorder"), R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("list.activeSelectionBackground", [H(0x04395eff), H(0x0060c0ff), N, H(0x0f4a851a)]),
    ("list.activeSelectionForeground", [H(0xffffffff), H(0xffffffff), N, N]),
    ("list.activeSelectionIconForeground", [N, N, N, N]),
    ("list.inactiveSelectionBackground", [H(0x37373dff), H(0xe4e6f1ff), N, H(0x0f4a851a)]),
    ("list.inactiveSelectionForeground", [N, N, N, N]),
    ("sash.hoverBorder", [R("focusBorder"), R("focusBorder"), R("focusBorder"), R("focusBorder")]),
    ("list.hoverBackground", [H(0x2a2d2eff), H(0xf0f0f0ff), H(0xffffff1a), H(0x0f4a851a)]),
    ("list.hoverForeground", [N, N, N, N]),
    ("list.highlightForeground", [H(0x2aaaffff), H(0x0066bfff), R("focusBorder"), R("focusBorder")]),
    ("list.focusHighlightForeground", [R("list.highlightForeground"), If("list.activeSelectionBackground", &R("list.highlightForeground"), &H(0xbbe7ffff)), R("list.highlightForeground"), R("list.highlightForeground")]),
    ("list.invalidItemForeground", [H(0xb89500ff), H(0xb89500ff), H(0xb89500ff), H(0xb5200dff)]),
    ("list.deemphasizedForeground", [H(0x8c8c8cff), H(0x8e8e90ff), H(0xa7a8a9ff), H(0x666666ff)]),
    ("editorActionList.background", [R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background")]),
    ("editorActionList.foreground", [R("editorWidget.foreground"), R("editorWidget.foreground"), R("editorWidget.foreground"), R("editorWidget.foreground")]),
    ("editorActionList.focusForeground", [R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground")]),
    ("editorActionList.focusBackground", [R("list.activeSelectionBackground"), R("list.activeSelectionBackground"), R("list.activeSelectionBackground"), R("list.activeSelectionBackground")]),
    ("menu.border", [N, N, R("contrastBorder"), R("contrastBorder")]),
    ("menu.foreground", [R("dropdown.foreground"), R("dropdown.foreground"), R("dropdown.foreground"), R("dropdown.foreground")]),
    ("menu.background", [R("dropdown.background"), R("dropdown.background"), R("dropdown.background"), R("dropdown.background")]),
    ("menu.selectionForeground", [R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground")]),
    ("menu.selectionBackground", [R("list.activeSelectionBackground"), R("list.activeSelectionBackground"), R("list.activeSelectionBackground"), R("list.activeSelectionBackground")]),
    ("menu.selectionBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("menu.separatorBackground", [T(&R("foreground"), 0.2), T(&R("foreground"), 0.2), R("contrastBorder"), R("contrastBorder")]),
    ("minimap.findMatchHighlight", [R("editor.findMatchHighlightBackground"), R("editor.findMatchHighlightBackground"), R("editor.findMatchHighlightBackground"), R("editor.findMatchHighlightBackground")]),
    ("minimap.selectionOccurrenceHighlight", [R("editor.selectionHighlightBackground"), R("editor.selectionHighlightBackground"), R("editor.selectionHighlightBackground"), R("editor.selectionHighlightBackground")]),
    ("minimap.selectionHighlight", [R("editor.selectionBackground"), R("editor.selectionBackground"), R("editor.selectionBackground"), R("editor.selectionBackground")]),
    ("minimap.infoHighlight", [R("editorInfo.foreground"), R("editorInfo.foreground"), R("editorInfo.border"), R("editorInfo.border")]),
    ("minimap.warningHighlight", [R("editorWarning.foreground"), R("editorWarning.foreground"), R("editorWarning.border"), R("editorWarning.border")]),
    ("minimap.errorHighlight", [H(0xff1212b3), H(0xff1212b3), H(0xff3232ff), H(0xb5200dff)]),
    ("minimap.background", [N, N, N, N]),
    ("minimap.foregroundOpacity", [H(0x000000ff), H(0x000000ff), H(0x000000ff), H(0x000000ff)]),
    ("minimapSlider.background", [T(&R("scrollbarSlider.background"), 0.5), T(&R("scrollbarSlider.background"), 0.5), T(&R("scrollbarSlider.background"), 0.5), T(&R("scrollbarSlider.background"), 0.5)]),
    ("minimapSlider.hoverBackground", [T(&R("scrollbarSlider.hoverBackground"), 0.5), T(&R("scrollbarSlider.hoverBackground"), 0.5), T(&R("scrollbarSlider.hoverBackground"), 0.5), T(&R("scrollbarSlider.hoverBackground"), 0.5)]),
    ("minimapSlider.activeBackground", [T(&R("scrollbarSlider.activeBackground"), 0.5), T(&R("scrollbarSlider.activeBackground"), 0.5), T(&R("scrollbarSlider.activeBackground"), 0.5), T(&R("scrollbarSlider.activeBackground"), 0.5)]),
    ("badge.background", [H(0x4d4d4dff), H(0xc4c4c4ff), H(0x000000ff), H(0x0f4a85ff)]),
    ("badge.foreground", [H(0xffffffff), H(0x333333ff), H(0xffffffff), H(0xffffffff)]),
    ("scrollbar.shadow", [H(0x000000ff), H(0xddddddff), N, N]),
    ("scrollbarSlider.background", [H(0x79797966), H(0x64646466), T(&R("contrastBorder"), 0.6), T(&R("contrastBorder"), 0.4)]),
    ("scrollbarSlider.hoverBackground", [H(0x646464b3), H(0x646464b3), T(&R("contrastBorder"), 0.8), T(&R("contrastBorder"), 0.8)]),
    ("scrollbarSlider.activeBackground", [H(0xbfbfbf66), H(0x00000099), R("contrastBorder"), R("contrastBorder")]),
    ("quickInput.background", [R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background")]),
    ("quickInput.foreground", [R("editorWidget.foreground"), R("editorWidget.foreground"), R("editorWidget.foreground"), R("editorWidget.foreground")]),
    ("quickInputTitle.background", [H(0xffffff1b), H(0x0000000f), H(0x000000ff), H(0xffffffff)]),
    ("progressBar.background", [H(0x0e70c0ff), H(0x0e70c0ff), R("contrastBorder"), R("contrastBorder")]),
    ("pickerGroup.foreground", [H(0x3794ffff), H(0x0066bfff), H(0xffffffff), H(0x0f4a85ff)]),
    ("pickerGroup.border", [H(0x3f3f46ff), H(0xcccedbff), H(0xffffffff), H(0x0f4a85ff)]),
    ("quickInput.list.focusBackground", [N, N, N, N]),
    ("quickInputList.focusForeground", [R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground"), R("list.activeSelectionForeground")]),
    ("quickInputList.focusIconForeground", [R("list.activeSelectionIconForeground"), R("list.activeSelectionIconForeground"), R("list.activeSelectionIconForeground"), R("list.activeSelectionIconForeground")]),
    ("quickInputList.focusBackground", [O(&[R("quickInput.list.focusBackground"), R("list.activeSelectionBackground")]), O(&[R("quickInput.list.focusBackground"), R("list.activeSelectionBackground")]), N, N]),
    ("quickInputList.focusHighlightForeground", [R("list.focusHighlightForeground"), R("list.focusHighlightForeground"), R("list.focusHighlightForeground"), R("list.focusHighlightForeground")]),
    ("symbolIcon.arrayForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.booleanForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.classForeground", [H(0xee9d28ff), H(0xd67e00ff), H(0xee9d28ff), H(0xd67e00ff)]),
    ("symbolIcon.colorForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.constantForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.constructorForeground", [H(0xb180d7ff), H(0x652d90ff), H(0xb180d7ff), H(0x652d90ff)]),
    ("symbolIcon.enumeratorForeground", [H(0xee9d28ff), H(0xd67e00ff), H(0xee9d28ff), H(0xd67e00ff)]),
    ("symbolIcon.enumeratorMemberForeground", [H(0x75beffff), H(0x007accff), H(0x75beffff), H(0x007accff)]),
    ("symbolIcon.eventForeground", [H(0xee9d28ff), H(0xd67e00ff), H(0xee9d28ff), H(0xd67e00ff)]),
    ("symbolIcon.fieldForeground", [H(0x75beffff), H(0x007accff), H(0x75beffff), H(0x007accff)]),
    ("symbolIcon.fileForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.folderForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.functionForeground", [H(0xb180d7ff), H(0x652d90ff), H(0xb180d7ff), H(0x652d90ff)]),
    ("symbolIcon.interfaceForeground", [H(0x75beffff), H(0x007accff), H(0x75beffff), H(0x007accff)]),
    ("symbolIcon.keyForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.keywordForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.methodForeground", [H(0xb180d7ff), H(0x652d90ff), H(0xb180d7ff), H(0x652d90ff)]),
    ("symbolIcon.moduleForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.namespaceForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.nullForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.numberForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.objectForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.operatorForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.packageForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.propertyForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.referenceForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.snippetForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.stringForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.structForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.textForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.typeParameterForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.unitForeground", [R("foreground"), R("foreground"), R("foreground"), R("foreground")]),
    ("symbolIcon.variableForeground", [H(0x75beffff), H(0x007accff), H(0x75beffff), H(0x007accff)]),
    ("editorSuggestWidget.background", [R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background"), R("editorWidget.background")]),
    ("editorSuggestWidget.border", [R("editorWidget.border"), R("editorWidget.border"), R("editorWidget.border"), R("editorWidget.border")]),
    ("editorSuggestWidget.foreground", [R("editor.foreground"), R("editor.foreground"), R("editor.foreground"), R("editor.foreground")]),
    ("editorSuggestWidget.selectedForeground", [R("quickInputList.focusForeground"), R("quickInputList.focusForeground"), R("editorSuggestWidget.background"), R("editorSuggestWidget.background")]),
    ("editorSuggestWidget.selectedIconForeground", [R("quickInputList.focusIconForeground"), R("quickInputList.focusIconForeground"), R("editorSuggestWidget.background"), R("editorSuggestWidget.background")]),
    ("editorSuggestWidget.selectedBackground", [R("quickInputList.focusBackground"), R("quickInputList.focusBackground"), R("editorSuggestWidget.foreground"), R("editorSuggestWidget.foreground")]),
    ("editorSuggestWidget.focusOutline", [R("contrastActiveBorder"), R("contrastActiveBorder"), R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editorSuggestWidget.highlightForeground", [R("list.highlightForeground"), R("list.highlightForeground"), R("list.highlightForeground"), R("list.highlightForeground")]),
    ("editorSuggestWidget.focusHighlightForeground", [R("list.focusHighlightForeground"), R("list.focusHighlightForeground"), R("editorSuggestWidget.selectedForeground"), R("editorSuggestWidget.selectedForeground")]),
    ("editorSuggestWidgetStatus.foreground", [T(&R("editorSuggestWidget.foreground"), 0.5), T(&R("editorSuggestWidget.foreground"), 0.5), T(&R("editorSuggestWidget.foreground"), 0.5), T(&R("editorSuggestWidget.foreground"), 0.5)]),
    ("editor.foldBackground", [T(&R("editor.selectionBackground"), 0.3), T(&R("editor.selectionBackground"), 0.3), N, N]),
    ("editor.foldPlaceholderForeground", [H(0x808080ff), H(0x808080ff), N, N]),
    ("editorGutter.foldingControlForeground", [R("icon.foreground"), R("icon.foreground"), R("icon.foreground"), R("icon.foreground")]),
    ("editor.placeholder.foreground", [R("editorGhostText.foreground"), R("editorGhostText.foreground"), R("editorGhostText.foreground"), R("editorGhostText.foreground")]),
    ("editorMarkerNavigationError.background", [O(&[R("editorError.foreground"), R("editorError.border")]), O(&[R("editorError.foreground"), R("editorError.border")]), R("contrastBorder"), R("contrastBorder")]),
    ("editorMarkerNavigationError.headerBackground", [T(&R("editorMarkerNavigationError.background"), 0.1), T(&R("editorMarkerNavigationError.background"), 0.1), N, N]),
    ("editorMarkerNavigationWarning.background", [O(&[R("editorWarning.foreground"), R("editorWarning.border")]), O(&[R("editorWarning.foreground"), R("editorWarning.border")]), R("contrastBorder"), R("contrastBorder")]),
    ("editorMarkerNavigationWarning.headerBackground", [T(&R("editorMarkerNavigationWarning.background"), 0.1), T(&R("editorMarkerNavigationWarning.background"), 0.1), H(0x0c141fff), T(&R("editorMarkerNavigationWarning.background"), 0.2)]),
    ("editorMarkerNavigationInfo.background", [O(&[R("editorInfo.foreground"), R("editorInfo.border")]), O(&[R("editorInfo.foreground"), R("editorInfo.border")]), R("contrastBorder"), R("contrastBorder")]),
    ("editorMarkerNavigationInfo.headerBackground", [T(&R("editorMarkerNavigationInfo.background"), 0.1), T(&R("editorMarkerNavigationInfo.background"), 0.1), N, N]),
    ("editorMarkerNavigation.background", [R("editor.background"), R("editor.background"), R("editor.background"), R("editor.background")]),
    ("peekViewTitle.background", [H(0x252526ff), H(0xf3f3f3ff), H(0x000000ff), H(0xffffffff)]),
    ("peekViewTitleLabel.foreground", [H(0xffffffff), H(0x000000ff), H(0xffffffff), R("editor.foreground")]),
    ("peekViewTitleDescription.foreground", [H(0xccccccb3), H(0x616161ff), H(0xffffff99), H(0x292929ff)]),
    ("peekView.border", [R("editorInfo.foreground"), R("editorInfo.foreground"), R("contrastBorder"), R("contrastBorder")]),
    ("peekViewResult.background", [H(0x252526ff), H(0xf3f3f3ff), H(0x000000ff), H(0xffffffff)]),
    ("peekViewResult.lineForeground", [H(0xbbbbbbff), H(0x646465ff), H(0xffffffff), R("editor.foreground")]),
    ("peekViewResult.fileForeground", [H(0xffffffff), H(0x1e1e1eff), H(0xffffffff), R("editor.foreground")]),
    ("peekViewResult.selectionBackground", [H(0x3399ff33), H(0x3399ff33), N, N]),
    ("peekViewResult.selectionForeground", [H(0xffffffff), H(0x6c6c6cff), H(0xffffffff), R("editor.foreground")]),
    ("peekViewEditor.background", [H(0x001f33ff), H(0xf2f8fcff), H(0x000000ff), H(0xffffffff)]),
    ("peekViewEditorGutter.background", [R("peekViewEditor.background"), R("peekViewEditor.background"), R("peekViewEditor.background"), R("peekViewEditor.background")]),
    ("peekViewEditorStickyScroll.background", [R("peekViewEditor.background"), R("peekViewEditor.background"), R("peekViewEditor.background"), R("peekViewEditor.background")]),
    ("peekViewEditorStickyScrollGutter.background", [R("peekViewEditor.background"), R("peekViewEditor.background"), R("peekViewEditor.background"), R("peekViewEditor.background")]),
    ("peekViewResult.matchHighlightBackground", [H(0xea5c004d), H(0xea5c004d), N, N]),
    ("peekViewEditor.matchHighlightBackground", [H(0xff8f0099), H(0xf5d802de), N, N]),
    ("peekViewEditor.matchHighlightBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editorOverviewRuler.bracketMatchForeground", [H(0xa0a0a0ff), H(0xa0a0a0ff), H(0xa0a0a0ff), H(0xa0a0a0ff)]),
    ("inlineEdit.originalBackground", [T(&R("diffEditor.removedTextBackground"), 0.2), T(&R("diffEditor.removedTextBackground"), 0.2), T(&R("diffEditor.removedTextBackground"), 0.2), T(&R("diffEditor.removedTextBackground"), 0.2)]),
    ("inlineEdit.modifiedBackground", [T(&R("diffEditor.insertedTextBackground"), 0.3), T(&R("diffEditor.insertedTextBackground"), 0.3), T(&R("diffEditor.insertedTextBackground"), 0.3), T(&R("diffEditor.insertedTextBackground"), 0.3)]),
    ("inlineEdit.originalChangedLineBackground", [T(&R("diffEditor.removedTextBackground"), 0.8), T(&R("diffEditor.removedTextBackground"), 0.8), T(&R("diffEditor.removedTextBackground"), 0.8), T(&R("diffEditor.removedTextBackground"), 0.8)]),
    ("inlineEdit.originalChangedTextBackground", [T(&R("diffEditor.removedTextBackground"), 0.8), T(&R("diffEditor.removedTextBackground"), 0.8), T(&R("diffEditor.removedTextBackground"), 0.8), T(&R("diffEditor.removedTextBackground"), 0.8)]),
    ("inlineEdit.modifiedChangedLineBackground", [T(&R("diffEditor.insertedLineBackground"), 0.7), T(&R("diffEditor.insertedLineBackground"), 0.7), R("diffEditor.insertedLineBackground"), R("diffEditor.insertedLineBackground")]),
    ("inlineEdit.modifiedChangedTextBackground", [T(&R("diffEditor.insertedTextBackground"), 0.7), T(&R("diffEditor.insertedTextBackground"), 0.7), T(&R("diffEditor.insertedTextBackground"), 0.7), T(&R("diffEditor.insertedTextBackground"), 0.7)]),
    ("inlineEdit.gutterIndicator.primaryForeground", [R("button.foreground"), R("button.foreground"), R("button.foreground"), R("button.foreground")]),
    ("inlineEdit.gutterIndicator.primaryBorder", [R("button.background"), R("button.background"), R("button.background"), R("button.background")]),
    ("inlineEdit.gutterIndicator.primaryBackground", [T(&R("inlineEdit.gutterIndicator.primaryBorder"), 0.4), T(&R("inlineEdit.gutterIndicator.primaryBorder"), 0.5), T(&R("inlineEdit.gutterIndicator.primaryBorder"), 0.4), T(&R("inlineEdit.gutterIndicator.primaryBorder"), 0.5)]),
    ("inlineEdit.gutterIndicator.secondaryForeground", [R("editorHoverWidget.foreground"), R("editorHoverWidget.foreground"), R("editorHoverWidget.foreground"), R("editorHoverWidget.foreground")]),
    ("inlineEdit.gutterIndicator.secondaryBorder", [R("editorHoverWidget.border"), R("editorHoverWidget.border"), R("editorHoverWidget.border"), R("editorHoverWidget.border")]),
    ("inlineEdit.gutterIndicator.secondaryBackground", [R("editorHoverWidget.background"), R("editorHoverWidget.background"), R("editorHoverWidget.background"), R("editorHoverWidget.background")]),
    ("inlineEdit.gutterIndicator.successfulForeground", [R("button.foreground"), R("button.foreground"), R("button.foreground"), R("button.foreground")]),
    ("inlineEdit.gutterIndicator.successfulBorder", [R("button.background"), R("button.background"), R("button.background"), R("button.background")]),
    ("inlineEdit.gutterIndicator.successfulBackground", [R("inlineEdit.gutterIndicator.successfulBorder"), R("inlineEdit.gutterIndicator.successfulBorder"), R("inlineEdit.gutterIndicator.successfulBorder"), R("inlineEdit.gutterIndicator.successfulBorder")]),
    ("inlineEdit.gutterIndicator.background", [T(&R("tab.inactiveBackground"), 0.5), H(0x5f5f5f18), T(&R("tab.inactiveBackground"), 0.5), T(&R("tab.inactiveBackground"), 0.5)]),
    ("inlineEdit.originalBorder", [R("diffEditor.removedTextBackground"), R("diffEditor.removedTextBackground"), R("diffEditor.removedTextBackground"), R("diffEditor.removedTextBackground")]),
    ("inlineEdit.modifiedBorder", [R("diffEditor.insertedTextBackground"), D(&R("diffEditor.insertedTextBackground"), 0.6), R("diffEditor.insertedTextBackground"), R("diffEditor.insertedTextBackground")]),
    ("inlineEdit.tabWillAcceptModifiedBorder", [D(&R("inlineEdit.modifiedBorder"), 0.0), D(&R("inlineEdit.modifiedBorder"), 0.0), D(&R("inlineEdit.modifiedBorder"), 0.0), D(&R("inlineEdit.modifiedBorder"), 0.0)]),
    ("inlineEdit.tabWillAcceptOriginalBorder", [D(&R("inlineEdit.originalBorder"), 0.0), D(&R("inlineEdit.originalBorder"), 0.0), D(&R("inlineEdit.originalBorder"), 0.0), D(&R("inlineEdit.originalBorder"), 0.0)]),
    ("editorHoverWidget.highlightForeground", [R("list.highlightForeground"), R("list.highlightForeground"), R("list.highlightForeground"), R("list.highlightForeground")]),
    ("editor.linkedEditingBackground", [H(0xff00004d), H(0xff00004d), H(0xff00004d), H(0xffffffff)]),
    ("editor.wordHighlightBackground", [H(0x575757b8), H(0x57575740), N, N]),
    ("editor.wordHighlightStrongBackground", [H(0x004972b8), H(0x0e639c40), N, N]),
    ("editor.wordHighlightTextBackground", [R("editor.wordHighlightBackground"), R("editor.wordHighlightBackground"), R("editor.wordHighlightBackground"), R("editor.wordHighlightBackground")]),
    ("editor.wordHighlightBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editor.wordHighlightStrongBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editor.wordHighlightTextBorder", [R("editor.wordHighlightBorder"), R("editor.wordHighlightBorder"), R("editor.wordHighlightBorder"), R("editor.wordHighlightBorder")]),
    ("editorOverviewRuler.wordHighlightForeground", [H(0xa0a0a0cc), H(0xa0a0a0cc), H(0xa0a0a0cc), H(0xa0a0a0cc)]),
    ("editorOverviewRuler.wordHighlightStrongForeground", [H(0xc0a0c0cc), H(0xc0a0c0cc), H(0xc0a0c0cc), H(0xc0a0c0cc)]),
    ("editorOverviewRuler.wordHighlightTextForeground", [R("editorOverviewRuler.selectionHighlightForeground"), R("editorOverviewRuler.selectionHighlightForeground"), R("editorOverviewRuler.selectionHighlightForeground"), R("editorOverviewRuler.selectionHighlightForeground")]),
    ("multiDiffEditor.headerBackground", [H(0x262626ff), R("tab.inactiveBackground"), R("tab.inactiveBackground"), R("tab.inactiveBackground")]),
    ("multiDiffEditor.border", [R("sideBarSectionHeader.border"), H(0xccccccff), R("sideBarSectionHeader.border"), H(0xccccccff)]),
    ("multiDiffEditor.background", [R("editor.background"), R("editor.background"), R("editor.background"), R("editor.background")]),
    ("diffEditor.move.border", [H(0x8b8b8b9c), H(0x8b8b8b9c), H(0x8b8b8b9c), H(0x8b8b8b9c)]),
    ("diffEditor.moveActive.border", [H(0xffa500ff), H(0xffa500ff), H(0xffa500ff), H(0xffa500ff)]),
    ("diffEditor.unchangedRegionShadow", [H(0x000000ff), H(0x737373bf), H(0x000000ff), H(0x737373bf)]),
    ("editor.lineHighlightBackground", [N, N, N, N]),
    ("editor.inactiveLineHighlightBackground", [R("editor.lineHighlightBackground"), R("editor.lineHighlightBackground"), R("editor.lineHighlightBackground"), R("editor.lineHighlightBackground")]),
    ("editor.lineHighlightBorder", [H(0x282828ff), H(0xeeeeeeff), H(0xf38518ff), R("contrastBorder")]),
    ("editor.rangeHighlightBackground", [H(0xffffff0b), H(0xfdff0033), N, N]),
    ("editor.rangeHighlightBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editor.symbolHighlightBackground", [R("editor.findMatchHighlightBackground"), R("editor.findMatchHighlightBackground"), N, N]),
    ("editor.symbolHighlightBorder", [N, N, R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editorCursor.foreground", [H(0xaeafadff), H(0x000000ff), H(0xffffffff), H(0x0f4a85ff)]),
    ("editorCursor.background", [N, N, N, N]),
    ("editorMultiCursor.primary.foreground", [R("editorCursor.foreground"), R("editorCursor.foreground"), R("editorCursor.foreground"), R("editorCursor.foreground")]),
    ("editorMultiCursor.primary.background", [R("editorCursor.background"), R("editorCursor.background"), R("editorCursor.background"), R("editorCursor.background")]),
    ("editorMultiCursor.secondary.foreground", [R("editorCursor.foreground"), R("editorCursor.foreground"), R("editorCursor.foreground"), R("editorCursor.foreground")]),
    ("editorMultiCursor.secondary.background", [R("editorCursor.background"), R("editorCursor.background"), R("editorCursor.background"), R("editorCursor.background")]),
    ("editorWhitespace.foreground", [H(0xe3e4e229), H(0x33333333), H(0xe3e4e229), H(0xccccccff)]),
    ("editorWordWrapIndicator.foreground", [R("editorWhitespace.foreground"), R("editorWhitespace.foreground"), R("editorWhitespace.foreground"), R("editorWhitespace.foreground")]),
    ("editorLineNumber.foreground", [H(0x858585ff), H(0x237893ff), H(0xffffffff), H(0x292929ff)]),
    ("editorIndentGuide.background", [R("editorWhitespace.foreground"), R("editorWhitespace.foreground"), R("editorWhitespace.foreground"), R("editorWhitespace.foreground")]),
    ("editorIndentGuide.activeBackground", [R("editorWhitespace.foreground"), R("editorWhitespace.foreground"), R("editorWhitespace.foreground"), R("editorWhitespace.foreground")]),
    ("editorIndentGuide.background1", [R("editorIndentGuide.background"), R("editorIndentGuide.background"), R("editorIndentGuide.background"), R("editorIndentGuide.background")]),
    ("editorIndentGuide.background2", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.background3", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.background4", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.background5", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.background6", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.activeBackground1", [R("editorIndentGuide.activeBackground"), R("editorIndentGuide.activeBackground"), R("editorIndentGuide.activeBackground"), R("editorIndentGuide.activeBackground")]),
    ("editorIndentGuide.activeBackground2", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.activeBackground3", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.activeBackground4", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.activeBackground5", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorIndentGuide.activeBackground6", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorActiveLineNumber.foreground", [H(0xc6c6c6ff), H(0x0b216fff), R("contrastActiveBorder"), R("contrastActiveBorder")]),
    ("editorLineNumber.activeForeground", [R("editorActiveLineNumber.foreground"), R("editorActiveLineNumber.foreground"), R("editorActiveLineNumber.foreground"), R("editorActiveLineNumber.foreground")]),
    ("editorLineNumber.dimmedForeground", [N, N, N, N]),
    ("editorRuler.foreground", [H(0x5a5a5aff), H(0xd3d3d3ff), H(0xffffffff), H(0x292929ff)]),
    ("editorCodeLens.foreground", [H(0x999999ff), H(0x919191ff), H(0x999999ff), H(0x292929ff)]),
    ("editor.inlineValuesForeground", [H(0xffffff80), H(0x00000080), H(0xffffff80), H(0x00000080)]),
    ("editor.inlineValuesBackground", [H(0xffc80033), H(0xffc80033), H(0xffc80033), H(0xffc80033)]),
    ("editorBracketMatch.background", [H(0x0064001a), H(0x0064001a), H(0x0064001a), H(0x00000000)]),
    ("editorBracketMatch.border", [H(0x888888ff), H(0xb9b9b9ff), R("contrastBorder"), R("contrastBorder")]),
    ("editorBracketMatch.foreground", [N, N, N, N]),
    ("editorOverviewRuler.border", [H(0x7f7f7f4d), H(0x7f7f7f4d), H(0x7f7f7f4d), H(0x666666ff)]),
    ("editorOverviewRuler.background", [N, N, N, N]),
    ("editorGutter.background", [R("editor.background"), R("editor.background"), R("editor.background"), R("editor.background")]),
    ("editorUnnecessaryCode.border", [N, N, H(0xffffffcc), R("contrastBorder")]),
    ("editorUnnecessaryCode.opacity", [H(0x000000aa), H(0x00000077), N, N]),
    ("editorGhostText.border", [N, N, H(0xffffffcc), H(0x292929cc)]),
    ("editorGhostText.foreground", [H(0xffffff56), H(0x00000077), N, N]),
    ("editorGhostText.background", [N, N, N, N]),
    ("editorOverviewRuler.rangeHighlightForeground", [H(0x007acc99), H(0x007acc99), H(0x007acc99), H(0x007acc99)]),
    ("editorOverviewRuler.errorForeground", [H(0xff1212b3), H(0xff1212b3), H(0xff3232ff), H(0xb5200dff)]),
    ("editorOverviewRuler.warningForeground", [R("editorWarning.foreground"), R("editorWarning.foreground"), R("editorWarning.border"), R("editorWarning.border")]),
    ("editorOverviewRuler.infoForeground", [R("editorInfo.foreground"), R("editorInfo.foreground"), R("editorInfo.border"), R("editorInfo.border")]),
    ("editorBracketHighlight.foreground1", [H(0xffd700ff), H(0x0431faff), H(0xffd700ff), H(0x0431faff)]),
    ("editorBracketHighlight.foreground2", [H(0xda70d6ff), H(0x319331ff), H(0xda70d6ff), H(0x319331ff)]),
    ("editorBracketHighlight.foreground3", [H(0x179fffff), H(0x7b3814ff), H(0x87cefaff), H(0x7b3814ff)]),
    ("editorBracketHighlight.foreground4", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketHighlight.foreground5", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketHighlight.foreground6", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketHighlight.unexpectedBracket.foreground", [H(0xff1212cc), H(0xff1212cc), H(0xff3232ff), H(0xb5200dff)]),
    ("editorBracketPairGuide.background1", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.background2", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.background3", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.background4", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.background5", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.background6", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.activeBackground1", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.activeBackground2", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.activeBackground3", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.activeBackground4", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.activeBackground5", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorBracketPairGuide.activeBackground6", [H(0x00000000), H(0x00000000), H(0x00000000), H(0x00000000)]),
    ("editorUnicodeHighlight.border", [R("editorWarning.foreground"), R("editorWarning.foreground"), R("editorWarning.foreground"), R("editorWarning.foreground")]),
    ("editorUnicodeHighlight.background", [R("editorWarning.background"), R("editorWarning.background"), R("editorWarning.background"), R("editorWarning.background")]),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn references(value: &V, out: &mut Vec<&'static str>) {
        match value {
            N | H(_) => {}
            R(id) => out.push(id),
            T(value, _) | D(value, _) | L(value, _) => references(value, out),
            O(values) => values.iter().for_each(|value| references(value, out)),
            LP(value, background, ..) => {
                references(value, out);
                references(background, out);
            }
            If(id, then, otherwise) => {
                out.push(id);
                references(then, out);
                references(otherwise, out);
            }
        }
    }

    #[test]
    fn defaults_only_refer_to_registered_colors() {
        // Workbench colors Monaco's standalone editor doesn't register
        // either: they resolve to nothing there too.
        let unregistered = ["sideBar.background", "quickInput.list.focusBackground"];
        let mut missing = Vec::new();
        for (id, defaults) in REGISTRY {
            let mut ids = Vec::new();
            defaults
                .iter()
                .for_each(|value| references(value, &mut ids));
            missing.extend(
                ids.into_iter()
                    .filter(|r| !is_registered(r) && !unregistered.contains(r))
                    .map(|r| format!("{id} -> {r}")),
            );
        }
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn transforms_match_monaco() {
        let colors = HashMap::new();
        let hex = |color: Hsla| {
            let color = to_color(color).rgba();
            format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
        };
        // `lighten(#252526, 0.2)` and `darken(#F3F3F3, 0.05)`: the hover
        // status bar.
        let mut dark = Resolver::new(ColorScheme::Dark, &colors);
        assert_eq!(
            hex(dark.color("editorHoverWidget.statusBarBackground")),
            "#2c2c2d"
        );
        let mut light = Resolver::new(ColorScheme::Light, &colors);
        assert_eq!(
            hex(light.color("editorHoverWidget.statusBarBackground")),
            "#e7e7e7"
        );
        // `darken(list.activeSelectionBackground, 0.3)`: the light banner.
        assert_eq!(hex(dark.color("banner.background")), "#04395e");
        assert_eq!(hex(light.color("banner.background")), "#004386");
        // `ifDefinedThenElse`: light `list.focusHighlightForeground` is
        // #BBE7FF unless the theme sets the selection background.
        assert_eq!(hex(light.color("list.focusHighlightForeground")), "#bbe7ff");
        let own = HashMap::from([(
            "list.activeSelectionBackground".to_string(),
            gpui::rgb(0x123456).into(),
        )]);
        let mut defined = Resolver::new(ColorScheme::Light, &own);
        assert_eq!(
            hex(defined.color("list.focusHighlightForeground")),
            "#0066bf"
        );
        // Null in high contrast, which outlines instead.
        let mut hc = Resolver::new(ColorScheme::HcDark, &colors);
        assert_eq!(hc.get("editor.findMatchHighlightBackground"), None);
        assert_eq!(
            hc.visible("editor.findMatchHighlightBorder"),
            hc.get("focusBorder")
        );
    }
}
