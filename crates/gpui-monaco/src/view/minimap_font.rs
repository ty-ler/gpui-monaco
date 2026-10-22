//! Monaco `MinimapCharRendererFactory.createSampleData`: the editor font
//! drawn white into a 10×16 cell per minimap character, as RGBA, for the
//! glyphs of minimap scales above 2.
//!
//! Monaco draws into a canvas. GPUI can't rasterize into a CPU buffer, so
//! this finds the font among the system's (`fontdb`) and rasterizes it
//! with `swash`, the rasterizer GPUI itself uses on Linux. Loading the
//! system font list takes a while, so callers run this in the background.

/// The sample sheet for `family` (a GPUI family name, or a CSS-like list
/// such as `Menlo, monospace`), or `None` when no listed font is
/// installed (or the `minimap-font` feature is off).
#[cfg(all(feature = "minimap-font", not(target_arch = "wasm32")))]
pub(crate) fn sample_data(family: &str) -> Option<Vec<u8>> {
    use super::minimap::sheet;
    use std::sync::OnceLock;
    use swash::scale::{Render, ScaleContext, Source};
    use swash::zeno::Format;

    static FONTS: OnceLock<fontdb::Database> = OnceLock::new();
    let db = FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    });
    let names: Vec<String> = family
        .split(',')
        .map(|name| name.trim().trim_matches(['"', '\'']).to_string())
        .filter(|name| !name.is_empty())
        .collect();
    let families: Vec<fontdb::Family> = names
        .iter()
        .map(|name| match name.as_str() {
            "monospace" => fontdb::Family::Monospace,
            "sans-serif" => fontdb::Family::SansSerif,
            "serif" => fontdb::Family::Serif,
            name => fontdb::Family::Name(name),
        })
        .collect();
    // Monaco draws `bold 16px <family>`.
    let id = db.query(&fontdb::Query {
        families: &families,
        weight: fontdb::Weight::BOLD,
        ..fontdb::Query::default()
    })?;
    // A family without a bold face is made bold, as browsers do.
    let synthetic_bold = db.face(id)?.weight.0 < 600;
    db.with_face_data(id, |data, index| {
        let font = swash::FontRef::from_index(data, index as usize)?;
        let size = sheet::SAMPLED_CHAR_HEIGHT as f32;
        let metrics = font.metrics(&[]).scale(size);
        // Canvas `textBaseline = 'middle'` at half the cell height: the
        // middle of the em square, whose ascent and descent are the font's
        // scaled to add up to the font size.
        let extent = (metrics.ascent + metrics.descent).max(f32::EPSILON);
        let baseline = size / 2. + size * (metrics.ascent - metrics.descent) / extent / 2.;
        let mut context = ScaleContext::new();
        let mut scaler = context.builder(font).size(size).hint(false).build();
        let mut render = Render::new(&[Source::Outline]);
        render.format(Format::Alpha);
        if synthetic_bold {
            render.embolden(size / 24.);
        }
        let (width, height) = (
            sheet::RGBA_SAMPLED_ROW_WIDTH / 4,
            sheet::SAMPLED_CHAR_HEIGHT,
        );
        let mut alpha = vec![0u8; width * height];
        for (i, ch) in sheet::all_chars().enumerate() {
            let glyph = font.charmap().map(ch);
            if glyph == 0 {
                continue;
            }
            let Some(image) = render.render(&mut scaler, glyph) else {
                continue;
            };
            let p = image.placement;
            let left = (i * sheet::SAMPLED_CHAR_WIDTH) as i32 + p.left;
            let top = baseline.floor() as i32 - p.top;
            for y in 0..p.height as i32 {
                let ty = top + y;
                if !(0..height as i32).contains(&ty) {
                    continue;
                }
                for x in 0..p.width as i32 {
                    let tx = left + x;
                    if !(0..width as i32).contains(&tx) {
                        continue;
                    }
                    let src = image.data[(y * p.width as i32 + x) as usize];
                    let dst = &mut alpha[ty as usize * width + tx as usize];
                    // Source-over, like successive `fillText`s.
                    *dst = (u32::from(src) + u32::from(*dst) * (255 - u32::from(src)) / 255)
                        .min(255) as u8;
                }
            }
        }
        Some(alpha.into_iter().flat_map(|a| [255, 255, 255, a]).collect())
    })?
}

#[cfg(not(all(feature = "minimap-font", not(target_arch = "wasm32"))))]
pub(crate) fn sample_data(_family: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(all(test, feature = "minimap-font", not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn samples_an_installed_monospace_font() {
        let Some(data) = sample_data("monospace") else {
            // No monospace font on this machine.
            return;
        };
        let sheet_width = super::super::minimap::sheet::RGBA_SAMPLED_ROW_WIDTH;
        assert_eq!(data.len(), sheet_width * 16);
        // Space is blank, `#` isn't.
        let cell_ink = |i: usize| {
            (0..16)
                .flat_map(|y| (0..10).map(move |x| y * sheet_width + (i * 10 + x) * 4 + 3))
                .map(|ix| u32::from(data[ix]))
                .sum::<u32>()
        };
        assert_eq!(cell_ink(0), 0);
        assert!(cell_ink(usize::from(b'#' - b' ')) > 0);
    }

    #[test]
    fn a_missing_font_has_no_sample() {
        assert!(sample_data("No Such Font Family 1234").is_none());
    }
}
