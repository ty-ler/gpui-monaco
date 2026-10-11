//! A port of Monaco's minimap (`browser/viewParts/minimap/`): the slider
//! and scroll geometry (`MinimapLayout`), line sampling
//! (`MinimapSamplingState`), the prebaked character bitmaps
//! (`minimapPreBaked.ts`, `minimapCharSheet.ts`, `MinimapCharRenderer`,
//! `MinimapCharRendererFactory`) and the line rasterizer
//! (`InnerMinimap._renderLine`).
//!
//! Everything here works in Monaco's units: line numbers are 1-based
//! minimap lines, canvas coordinates are device pixels of the canvas's
//! backing store, and slider positions are logical pixels.

use std::sync::{Arc, Mutex, OnceLock};

/// Monaco `MINIMAP_GUTTER_WIDTH`: device pixels left of the text.
pub(crate) const MINIMAP_GUTTER_WIDTH: f64 = 8.;

/// Monaco's `GUTTER_DECORATION_WIDTH`: the width of a
/// [`MinimapPosition::Gutter`](crate::MinimapPosition) mark, in device
/// pixels, 2px from the left.
pub(crate) const GUTTER_DECORATION_WIDTH: f64 = 2.;

/// Monaco `POINTER_DRAG_RESET_DISTANCE`: on Windows, dragging the slider
/// this far sideways puts the scroll position back.
pub(crate) const POINTER_DRAG_RESET_DISTANCE: f32 = 140.;

/// Monaco `minimapCharSheet.ts` `Constants`.
mod sheet {
    pub const START_CH_CODE: i32 = 32;
    pub const END_CH_CODE: i32 = 126;
    pub const CHAR_COUNT: i32 = END_CH_CODE - START_CH_CODE + 2;
    pub const SAMPLED_CHAR_HEIGHT: usize = 16;
    pub const SAMPLED_CHAR_WIDTH: usize = 10;
    pub const BASE_CHAR_HEIGHT: u32 = 2;
    pub const BASE_CHAR_WIDTH: u32 = 1;
    pub const RGBA_CHANNELS_CNT: usize = 4;
    pub const RGBA_SAMPLED_ROW_WIDTH: usize =
        RGBA_CHANNELS_CNT * CHAR_COUNT as usize * SAMPLED_CHAR_WIDTH;
}

/// Monaco `getCharIndex`: the glyph for UTF-16 code unit `ch_code`.
pub(crate) fn char_index(ch_code: u32, font_scale: u32) -> usize {
    let code = ch_code as i32 - sheet::START_CH_CODE;
    if !(0..=sheet::CHAR_COUNT).contains(&code) {
        if font_scale <= 2 {
            // For smaller scales, any ASCII character will do.
            return ((code + sheet::CHAR_COUNT).rem_euclid(sheet::CHAR_COUNT)) as usize;
        }
        // The unknown symbol.
        return (sheet::CHAR_COUNT - 1) as usize;
    }
    code as usize
}

/// Monaco `RGBA8`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    /// From a GPUI color, with Monaco's `Math.round(255 * a)` alpha.
    pub fn from_hsla(color: gpui::Hsla) -> Self {
        let rgba = gpui::Rgba::from(color);
        Self {
            r: clamped(f64::from(rgba.r) * 255.),
            g: clamped(f64::from(rgba.g) * 255.),
            b: clamped(f64::from(rgba.b) * 255.),
            a: (f64::from(rgba.a) * 255.).round().clamp(0., 255.) as u8,
        }
    }
}

/// A `Uint8ClampedArray` store: clamped and rounded half to even.
fn clamped(value: f64) -> u8 {
    if value.is_nan() {
        return 0;
    }
    value.clamp(0., 255.).round_ties_even() as u8
}

/// Monaco `toUint8`: truncated into 0..=255.
fn to_uint8(value: f64) -> u8 {
    if value < 0. {
        0
    } else if value > 255. {
        255
    } else {
        value as u8
    }
}

/// A canvas `ImageData`: RGBA bytes, row by row.
#[derive(Clone, Debug)]
pub(crate) struct ImageData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl ImageData {
    /// An image filled with `background` (Monaco `MinimapBuffers`).
    pub fn filled(width: u32, height: u32, background: Rgba8) -> Self {
        let data = [background.r, background.g, background.b, background.a]
            .repeat((width * height) as usize);
        Self {
            width,
            height,
            data,
        }
    }
}

/// Monaco `minimapPreBaked.ts` `decodeData`.
fn decode_data(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            (digit(pair[0]) << 4) | digit(pair[1])
        })
        .collect()
}

/// Monaco `prebakedMiniMaps[1]`.
const PREBAKED_1: &str = "0000511D6300CF609C709645A78432005642574171487021003C451900274D35D762755E8B629C5BA856AF57BA649530C167D1512A272A3F6038604460398526BCA2A968DB6F8957C768BE5FBE2FB467CF5D8D5B795DC7625B5DFF50DE64C466DB2FC47CD860A65E9A2EB96CB54CE06DA763AB2EA26860524D3763536601005116008177A8705E53AB738E6A982F88BAA35B5F5B626D9C636B449B737E5B7B678598869A662F6B5B8542706C704C80736A607578685B70594A49715A4522E792";

/// Monaco `prebakedMiniMaps[2]`.
const PREBAKED_2: &str = "000000000000000055394F383D2800008B8B1F210002000081B1CBCBCC820000847AAF6B9AAF2119BE08B8881AD60000A44FD07DCCF107015338130C00000000385972265F390B406E2437634B4B48031B12B8A0847000001E15B29A402F0000000000004B33460B00007A752C2A0000000000004D3900000084394B82013400ABA5CFC7AD9C0302A45A3E5A98AB000089A43382D97900008BA54AA087A70A0248A6A7AE6DBE0000BF6F94987EA40A01A06DCFA7A7A9030496C32F77891D0000A99FB1A0AFA80603B29AB9CA75930D010C0948354D3900000C0948354F37460D0028BE673D8400000000AF9D7B6E00002B007AA8933400007AA642675C2700007984CFB9C3985B768772A8A6B7B20000CAAECAAFC4B700009F94A6009F840009D09F9BA4CA9C0000CC8FC76DC87F0000C991C472A2000000A894A48CA7B501079BA2C9C69BA20000B19A5D3FA89000005CA6009DA2960901B0A7F0669FB200009D009E00B7890000DAD0F5D092820000D294D4C48BD10000B5A7A4A3B1A50402CAB6CBA6A2000000B5A7A4A3B1A8044FCDADD19D9CB00000B7778F7B8AAE0803C9AB5D3F5D3F00009EA09EA0BAB006039EA0989A8C7900009B9EF4D6B7C00000A9A7816CACA80000ABAC84705D3F000096DA635CDC8C00006F486F266F263D4784006124097B00374F6D2D6D2D6D4A3A95872322000000030000000000008D8939130000000000002E22A5C9CBC70600AB25C0B5C9B400061A2DB04CA67001082AA6BEBEBFC606002321DACBC19E03087AA08B6768380000282FBAC0B8CA7A88AD25BBA5A29900004C396C5894A6000040485A6E356E9442A32CD17EADA70000B4237923628600003E2DE9C1D7B500002F25BBA5A2990000231DB6AFB4A804023025C0B5CAB588062B2CBDBEC0C706882435A75CA20000002326BD6A82A908048B4B9A5A668000002423A09CB4BB060025259C9D8A7900001C1FCAB2C7C700002A2A9387ABA200002626A4A47D6E9D14333163A0C87500004B6F9C2D643A257049364936493647358A34438355497F1A0000A24C1D590000D38DFFBDD4CD3126";

/// Monaco `MinimapCharRenderer`: draws characters from per-pixel glyph
/// intensities, softened by 12/15 (by 50/60 on light themes).
#[derive(Debug)]
pub(crate) struct CharRenderer {
    pub scale: u32,
    normal: Vec<u8>,
    light: Vec<u8>,
}

impl CharRenderer {
    fn new(char_data: &[u8], scale: u32) -> Self {
        let soften = |ratio: f64| {
            char_data
                .iter()
                .map(|&v| to_uint8(f64::from(v) * ratio))
                .collect()
        };
        Self {
            scale,
            normal: soften(12. / 15.),
            light: soften(50. / 60.),
        }
    }

    /// Monaco `MinimapCharRendererFactory.create`: the prebaked glyphs at
    /// scales 1 and 2. Monaco samples the editor font at larger scales;
    /// GPUI has no way to rasterize a font into a buffer, so those scale up
    /// the scale-2 glyphs through Monaco's own downsampler instead.
    pub fn create(scale: u32) -> Arc<Self> {
        static CACHE: OnceLock<Mutex<Vec<Arc<CharRenderer>>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(renderer) = cache.iter().find(|r| r.scale == scale) {
            return renderer.clone();
        }
        let scale = scale.max(1);
        let renderer = Arc::new(match scale {
            1 => Self::new(&decode_data(PREBAKED_1), 1),
            2 => Self::new(&decode_data(PREBAKED_2), 2),
            _ => Self::from_sample_data(&upscaled_sample_data(), scale),
        });
        cache.push(renderer.clone());
        renderer
    }

    /// Monaco `MinimapCharRendererFactory.createFromSampleData`: glyphs
    /// downsampled from 10×16 RGBA samples of each character.
    pub fn from_sample_data(source: &[u8], scale: u32) -> Self {
        Self::new(&downsample(source, scale), scale)
    }

    /// Monaco `renderChar`: blends a character's glyph from `background`
    /// to `color` at `(dx, dy)`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_char(
        &self,
        target: &mut ImageData,
        dx: u32,
        dy: u32,
        ch_code: u32,
        color: Rgba8,
        foreground_alpha: u8,
        background: Rgba8,
        background_alpha: u8,
        font_scale: u32,
        use_lighter_font: bool,
        force_1px_height: bool,
    ) {
        let char_width = sheet::BASE_CHAR_WIDTH * self.scale;
        let char_height = sheet::BASE_CHAR_HEIGHT * self.scale;
        let render_height = if force_1px_height { 1 } else { char_height };
        if dx + char_width > target.width || dy + render_height > target.height {
            return;
        }
        let char_data = if use_lighter_font {
            &self.light
        } else {
            &self.normal
        };
        let index = char_index(ch_code, font_scale);
        let dest_width = target.width as usize * 4;
        let (br, bg, bb) = (
            f64::from(background.r),
            f64::from(background.g),
            f64::from(background.b),
        );
        let (dr, dg, db) = (
            f64::from(color.r) - br,
            f64::from(color.g) - bg,
            f64::from(color.b) - bb,
        );
        let dest_alpha = foreground_alpha.max(background_alpha);
        let mut source = index * (char_width * char_height) as usize;
        let mut row = dy as usize * dest_width + dx as usize * 4;
        for _ in 0..render_height {
            let mut column = row;
            for _ in 0..char_width {
                let c = f64::from(char_data.get(source).copied().unwrap_or(0)) / 255.
                    * (f64::from(foreground_alpha) / 255.);
                source += 1;
                let dest = &mut target.data[column..column + 4];
                dest[0] = clamped(br + dr * c);
                dest[1] = clamped(bg + dg * c);
                dest[2] = clamped(bb + db * c);
                dest[3] = dest_alpha;
                column += 4;
            }
            row += dest_width;
        }
    }

    /// Monaco `blockRenderChar`: a solid block at half the color.
    #[allow(clippy::too_many_arguments)]
    pub fn block_render_char(
        &self,
        target: &mut ImageData,
        dx: u32,
        dy: u32,
        color: Rgba8,
        foreground_alpha: u8,
        background: Rgba8,
        background_alpha: u8,
        force_1px_height: bool,
    ) {
        let char_width = sheet::BASE_CHAR_WIDTH * self.scale;
        let char_height = sheet::BASE_CHAR_HEIGHT * self.scale;
        let render_height = if force_1px_height { 1 } else { char_height };
        if dx + char_width > target.width || dy + render_height > target.height {
            return;
        }
        let dest_width = target.width as usize * 4;
        let c = 0.5 * (f64::from(foreground_alpha) / 255.);
        let blend = |bg: u8, fg: u8| clamped(f64::from(bg) + (f64::from(fg) - f64::from(bg)) * c);
        let pixel = [
            blend(background.r, color.r),
            blend(background.g, color.g),
            blend(background.b, color.b),
            foreground_alpha.max(background_alpha),
        ];
        let mut row = dy as usize * dest_width + dx as usize * 4;
        for _ in 0..render_height {
            for x in 0..char_width as usize {
                target.data[row + x * 4..row + x * 4 + 4].copy_from_slice(&pixel);
            }
            row += dest_width;
        }
    }
}

/// Monaco `MinimapCharRendererFactory._downsample`.
fn downsample(data: &[u8], scale: u32) -> Vec<u8> {
    let pixels_per_character =
        (sheet::BASE_CHAR_HEIGHT * scale * sheet::BASE_CHAR_WIDTH * scale) as usize;
    let result_len = pixels_per_character * sheet::CHAR_COUNT as usize;
    let mut result = vec![0u8; result_len];
    let mut result_offset = 0;
    let mut source_offset = 0;
    let mut brightest: f64 = 0.;
    for _ in 0..sheet::CHAR_COUNT {
        brightest = brightest.max(downsample_char(
            data,
            source_offset,
            &mut result,
            result_offset,
            scale,
        ));
        result_offset += pixels_per_character;
        source_offset += sheet::SAMPLED_CHAR_WIDTH * sheet::RGBA_CHANNELS_CNT;
    }
    if brightest > 0. {
        let adjust = 255. / brightest;
        for value in &mut result {
            *value = clamped(f64::from(*value) * adjust);
        }
    }
    result
}

/// Monaco `MinimapCharRendererFactory._downsampleChar`.
fn downsample_char(
    source: &[u8],
    source_offset: usize,
    dest: &mut [u8],
    dest_offset: usize,
    scale: u32,
) -> f64 {
    let width = (sheet::BASE_CHAR_WIDTH * scale) as usize;
    let height = (sheet::BASE_CHAR_HEIGHT * scale) as usize;
    let mut target = dest_offset;
    let mut brightest: f64 = 0.;
    for y in 0..height {
        let source_y1 = (y as f64 / height as f64) * sheet::SAMPLED_CHAR_HEIGHT as f64;
        let source_y2 = ((y + 1) as f64 / height as f64) * sheet::SAMPLED_CHAR_HEIGHT as f64;
        for x in 0..width {
            let source_x1 = (x as f64 / width as f64) * sheet::SAMPLED_CHAR_WIDTH as f64;
            let source_x2 = ((x + 1) as f64 / width as f64) * sheet::SAMPLED_CHAR_WIDTH as f64;
            let mut value = 0.;
            let mut samples = 0.;
            let mut sy = source_y1;
            while sy < source_y2 {
                let source_row =
                    source_offset + sy.floor() as usize * sheet::RGBA_SAMPLED_ROW_WIDTH;
                let y_balance = 1. - (sy - sy.floor());
                let mut sx = source_x1;
                while sx < source_x2 {
                    let x_balance = 1. - (sx - sx.floor());
                    let index = source_row + sx.floor() as usize * sheet::RGBA_CHANNELS_CNT;
                    let weight = x_balance * y_balance;
                    samples += weight;
                    let channel = |i: usize| f64::from(source.get(i).copied().unwrap_or(0));
                    value += channel(index) * channel(index + 3) / 255. * weight;
                    sx += 1.;
                }
                sy += 1.;
            }
            let fin = value / samples;
            brightest = brightest.max(fin);
            dest[target] = to_uint8(fin);
            target += 1;
        }
    }
    brightest
}

/// A stand-in for Monaco's font samples (`createSampleData`, which draws
/// the font into a canvas): the scale-2 glyphs, each pixel stretched over
/// a 5×4 block of the 10×16 sample cell.
fn upscaled_sample_data() -> Vec<u8> {
    let glyphs = decode_data(PREBAKED_2);
    let (gw, gh) = (2, 4);
    let mut data = vec![0u8; sheet::SAMPLED_CHAR_HEIGHT * sheet::RGBA_SAMPLED_ROW_WIDTH];
    for ch in 0..sheet::CHAR_COUNT as usize {
        for y in 0..sheet::SAMPLED_CHAR_HEIGHT {
            for x in 0..sheet::SAMPLED_CHAR_WIDTH {
                let value = glyphs[ch * gw * gh
                    + (y * gh / sheet::SAMPLED_CHAR_HEIGHT) * gw
                    + x * gw / sheet::SAMPLED_CHAR_WIDTH];
                let i = y * sheet::RGBA_SAMPLED_ROW_WIDTH
                    + (ch * sheet::SAMPLED_CHAR_WIDTH + x) * sheet::RGBA_CHANNELS_CNT;
                data[i..i + 4].copy_from_slice(&[255, 255, 255, value]);
            }
        }
    }
    data
}

/// What [`MinimapLayout::create`] needs from the minimap options (Monaco
/// `MinimapOptions`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayoutOptions {
    pub pixel_ratio: f64,
    /// Device pixels per minimap line.
    pub minimap_line_height: f64,
    pub canvas_inner_height: f64,
    pub line_height: f64,
    pub minimap_height_is_editor_height: bool,
    pub scroll_beyond_last_line: bool,
    pub padding_top: f64,
    pub padding_bottom: f64,
    /// The minimap's height in logical pixels.
    pub minimap_height: f64,
}

/// What [`MinimapLayout::create`] needs from the frame (Monaco
/// `IMinimapRenderingContext`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayoutInput {
    /// The first and last (partially) visible lines, 1-based, as minimap
    /// lines.
    pub viewport_start_line_number: u32,
    pub viewport_end_line_number: u32,
    /// The content y of `viewport_start_line_number`'s top.
    pub viewport_start_line_number_vertical_offset: f64,
    pub viewport_height: f64,
    /// View zones show in the viewport.
    pub viewport_contains_whitespace_gaps: bool,
    /// Minimap lines (sampled lines when sampling).
    pub line_count: u32,
    /// View lines.
    pub real_line_count: u32,
    pub scroll_top: f64,
    pub scroll_height: f64,
}

/// Monaco `MinimapLayout`: where the slider is and which lines the minimap
/// shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MinimapLayout {
    pub scroll_top: f64,
    pub scroll_height: f64,
    pub slider_needed: bool,
    computed_slider_ratio: f64,
    /// The slider's top, in logical pixels from the minimap's top.
    pub slider_top: f64,
    /// The slider's height, in logical pixels.
    pub slider_height: f64,
    /// Empty lines to reserve at the top of the minimap.
    pub top_padding_line_count: u32,
    /// The first and last line drawn (1-based minimap lines).
    pub start_line_number: u32,
    pub end_line_number: u32,
}

impl MinimapLayout {
    /// Monaco `getDesiredScrollTopFromDelta`: the scroll position that
    /// moves the slider by `delta` from this layout's.
    pub fn desired_scroll_top_from_delta(&self, delta: f64) -> f64 {
        (self.scroll_top + delta / self.computed_slider_ratio).round()
    }

    /// Monaco `intersectWithViewport`.
    pub fn intersect_with_viewport(&self, start: u32, end: u32) -> Option<(u32, u32)> {
        let start = self.start_line_number.max(start);
        let end = self.end_line_number.min(end);
        (start <= end).then_some((start, end))
    }

    /// Monaco `getYForLineNumber`: canvas y of a line's top.
    pub fn y_for_line_number(&self, line_number: u32, minimap_line_height: f64) -> f64 {
        (f64::from(line_number) - f64::from(self.start_line_number)
            + f64::from(self.top_padding_line_count))
            * minimap_line_height
    }

    /// Monaco `MinimapLayout.create`.
    pub fn create(
        options: &LayoutOptions,
        input: &LayoutInput,
        previous: Option<&MinimapLayout>,
    ) -> Self {
        let pixel_ratio = options.pixel_ratio;
        let minimap_line_height = options.minimap_line_height;
        let minimap_lines_fitting = (options.canvas_inner_height / minimap_line_height).floor();
        let line_height = options.line_height;
        let viewport_height = input.viewport_height;
        let scroll_top = input.scroll_top;
        let scroll_height = input.scroll_height;
        let line_count = f64::from(input.line_count);

        if options.minimap_height_is_editor_height {
            let mut logical_scroll_height = f64::from(input.real_line_count) * line_height
                + options.padding_top
                + options.padding_bottom;
            if options.scroll_beyond_last_line {
                logical_scroll_height +=
                    (viewport_height - line_height - options.padding_bottom).max(0.);
            }
            let slider_height = (viewport_height * viewport_height / logical_scroll_height)
                .floor()
                .max(1.);
            let max_minimap_slider_top = (options.minimap_height - slider_height).max(0.);
            // The slider moves from 0 to `max_minimap_slider_top` as the
            // scroll position does from 0 to `scroll_height - viewport_height`.
            let computed_slider_ratio = max_minimap_slider_top / (scroll_height - viewport_height);
            let slider_top = scroll_top * computed_slider_ratio;
            let max_lines_fitting = (options.canvas_inner_height / minimap_line_height).floor();
            let top_padding_line_count = (options.padding_top / line_height).floor();
            return Self {
                scroll_top,
                scroll_height,
                slider_needed: max_minimap_slider_top > 0.,
                computed_slider_ratio,
                slider_top,
                slider_height,
                top_padding_line_count: top_padding_line_count as u32,
                start_line_number: 1,
                end_line_number: line_count.min(max_lines_fitting) as u32,
            };
        }

        // The visible line count changes with partially visible lines,
        // view zones and scrolling beyond the last line, so first pick a
        // slider height.
        let slider_height = if input.viewport_contains_whitespace_gaps
            && input.viewport_end_line_number != input.line_count
        {
            // View zones show: the slider reflects the visible lines.
            let viewport_line_count = f64::from(input.viewport_end_line_number)
                - f64::from(input.viewport_start_line_number)
                + 1.;
            (viewport_line_count * minimap_line_height / pixel_ratio).floor()
        } else {
            // A stable height.
            let expected_viewport_line_count = viewport_height / line_height;
            (expected_viewport_line_count * minimap_line_height / pixel_ratio).floor()
        };

        let extra_lines_at_the_top = (options.padding_top / line_height).floor();
        let mut extra_lines_at_the_bottom = (options.padding_bottom / line_height).floor();
        if options.scroll_beyond_last_line {
            let expected_viewport_line_count = viewport_height / line_height;
            extra_lines_at_the_bottom =
                extra_lines_at_the_bottom.max(expected_viewport_line_count - 1.);
        }

        let mut max_minimap_slider_top = if extra_lines_at_the_bottom > 0. {
            let expected_viewport_line_count = viewport_height / line_height;
            // Dragged all the way down, the slider has the last line at its
            // top.
            (extra_lines_at_the_top + line_count + extra_lines_at_the_bottom
                - expected_viewport_line_count
                - 1.)
                * minimap_line_height
                / pixel_ratio
        } else {
            // Dragged all the way down, the slider has the last line at its
            // bottom.
            ((extra_lines_at_the_top + line_count) * minimap_line_height / pixel_ratio
                - slider_height)
                .max(0.)
        };
        max_minimap_slider_top = max_minimap_slider_top.min(options.minimap_height - slider_height);

        let computed_slider_ratio = max_minimap_slider_top / (scroll_height - viewport_height);
        let slider_top = scroll_top * computed_slider_ratio;

        if minimap_lines_fitting >= extra_lines_at_the_top + line_count + extra_lines_at_the_bottom
        {
            // All lines fit.
            return Self {
                scroll_top,
                scroll_height,
                slider_needed: max_minimap_slider_top > 0.,
                computed_slider_ratio,
                slider_top,
                slider_height,
                top_padding_line_count: extra_lines_at_the_top as u32,
                start_line_number: 1,
                end_line_number: input.line_count,
            };
        }

        let considering_start_line_number = if input.viewport_start_line_number > 1 {
            f64::from(input.viewport_start_line_number) + extra_lines_at_the_top
        } else {
            (scroll_top / line_height).max(1.)
        };
        let mut start_line_number = (considering_start_line_number
            - slider_top * pixel_ratio / minimap_line_height)
            .floor()
            .max(1.);
        let mut top_padding_line_count;
        if start_line_number < extra_lines_at_the_top {
            top_padding_line_count = extra_lines_at_the_top - start_line_number + 1.;
            start_line_number = 1.;
        } else {
            top_padding_line_count = 0.;
            start_line_number = (start_line_number - extra_lines_at_the_top).max(1.);
        }

        // Avoid flickering from a partially visible first line: stay
        // consistent with the previous layout.
        if let Some(previous) = previous
            && previous.scroll_height == scroll_height
        {
            if previous.scroll_top > scroll_top {
                // Scrolling up: never move the start down.
                start_line_number = start_line_number.min(f64::from(previous.start_line_number));
                top_padding_line_count =
                    top_padding_line_count.max(f64::from(previous.top_padding_line_count));
            }
            if previous.scroll_top < scroll_top {
                // Scrolling down: never move the start up.
                start_line_number = start_line_number.max(f64::from(previous.start_line_number));
                top_padding_line_count =
                    top_padding_line_count.min(f64::from(previous.top_padding_line_count));
            }
        }

        let end_line_number =
            line_count.min(start_line_number - top_padding_line_count + minimap_lines_fitting - 1.);
        let partial_line =
            (scroll_top - input.viewport_start_line_number_vertical_offset) / line_height;

        let slider_top_aligned = if scroll_top >= options.padding_top {
            (f64::from(input.viewport_start_line_number) - start_line_number
                + top_padding_line_count
                + partial_line)
                * minimap_line_height
                / pixel_ratio
        } else {
            (scroll_top / options.padding_top)
                * (top_padding_line_count + partial_line)
                * minimap_line_height
                / pixel_ratio
        };

        Self {
            scroll_top,
            scroll_height,
            slider_needed: true,
            computed_slider_ratio,
            slider_top: slider_top_aligned,
            slider_height,
            top_padding_line_count: top_padding_line_count as u32,
            start_line_number: start_line_number as u32,
            end_line_number: end_line_number.max(0.) as u32,
        }
    }
}

/// Monaco `MinimapSamplingState`: which view line each minimap line shows
/// when there are more lines than pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SamplingState {
    pub sampling_ratio: f64,
    /// 1-based view line numbers, by 0-based minimap line.
    pub minimap_lines: Vec<u32>,
}

impl SamplingState {
    /// Monaco `MinimapSamplingState.compute` without a previous state.
    pub fn compute(view_line_count: u32, minimap_line_count: f64) -> Self {
        let minimap_line_count = minimap_line_count.max(1.) as usize;
        let ratio = f64::from(view_line_count) / minimap_line_count as f64;
        let half_ratio = ratio / 2.;
        let mut result = vec![1u32; minimap_line_count];
        if minimap_line_count > 1 {
            for (i, line) in result.iter_mut().enumerate().take(minimap_line_count - 1) {
                *line = (i as f64 * ratio + half_ratio).round() as u32;
            }
            result[minimap_line_count - 1] = view_line_count;
        }
        result[0] = 1;
        Self {
            sampling_ratio: ratio,
            minimap_lines: result,
        }
    }

    /// Monaco `modelLineToMinimapLine`.
    pub fn model_line_to_minimap_line(&self, line_number: u32) -> u32 {
        ((f64::from(line_number) / self.sampling_ratio)
            .round()
            .max(1.) as u32)
            .min(self.minimap_lines.len() as u32)
    }

    /// Monaco `decorationLineRangeToMinimapLineRange`.
    pub fn decoration_line_range_to_minimap_line_range(&self, start: u32, end: u32) -> (u32, u32) {
        let mut minimap_start = self.model_line_to_minimap_line(start);
        let mut minimap_end = self.model_line_to_minimap_line(end);
        if start != end && minimap_end == minimap_start {
            if minimap_end as usize == self.minimap_lines.len() {
                if minimap_start > 1 {
                    minimap_start -= 1;
                }
            } else {
                minimap_end += 1;
            }
        }
        (minimap_start, minimap_end)
    }
}

/// How lines are drawn (Monaco `MinimapOptions`' rendering fields).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineStyle {
    /// Monaco `RenderMinimap.Text` (else blocks).
    pub text: bool,
    pub font_scale: u32,
    /// Device pixels per character column.
    pub char_width: u32,
    pub minimap_line_height: f64,
    /// The background characters blend from (`minimap.background` over the
    /// editor background) and the canvas background's alpha.
    pub render_background: Rgba8,
    pub background_alpha: u8,
    /// Monaco `minimap.foregroundOpacity`, 0–255.
    pub foreground_alpha: u8,
    /// The theme's background is light (Monaco `backgroundIsLight`).
    pub use_lighter_font: bool,
    pub tab_size: u32,
}

/// Monaco `InnerMinimap._renderLine`: one line's UTF-16 `content`, colored
/// by `color_at(utf16 index)`, at canvas row `dy`.
pub(crate) fn render_line(
    target: &mut ImageData,
    renderer: &CharRenderer,
    style: &LineStyle,
    dy: f64,
    content: &[u16],
    mut color_at: impl FnMut(usize) -> Rgba8,
) {
    let char_width = style.char_width;
    let base_char_height = if style.text {
        sheet::BASE_CHAR_HEIGHT
    } else {
        sheet::BASE_CHAR_HEIGHT + 1
    };
    let render_line_height = f64::from(base_char_height * style.font_scale);
    let inner_line_padding = if style.minimap_line_height > render_line_height {
        ((style.minimap_line_height - render_line_height) / 2.).floor()
    } else {
        0.
    };
    let dy = (dy + inner_line_padding).floor().max(0.) as u32;
    let max_dx = target.width as i64 - i64::from(char_width);
    let force_1px_height = style.minimap_line_height == 1.;
    let tab_size = style.tab_size.max(1) as usize;

    let mut dx = MINIMAP_GUTTER_WIDTH as i64;
    let mut tabs_char_delta = 0usize;
    for (char_index, &code) in content.iter().enumerate() {
        if dx > max_dx {
            // The minimap's edge.
            return;
        }
        if code == u16::from(b'\t') {
            let insert_spaces_count = tab_size - (char_index + tabs_char_delta) % tab_size;
            tabs_char_delta += insert_spaces_count - 1;
            dx += insert_spaces_count as i64 * i64::from(char_width);
        } else if code == u16::from(b' ') {
            dx += i64::from(char_width);
        } else {
            // Full width characters take two cells.
            let count = if crate::cursor::strings::is_full_width_character(u32::from(code)) {
                2
            } else {
                1
            };
            let color = color_at(char_index);
            for _ in 0..count {
                if style.text {
                    renderer.render_char(
                        target,
                        dx as u32,
                        dy,
                        u32::from(code),
                        color,
                        style.foreground_alpha,
                        style.render_background,
                        style.background_alpha,
                        style.font_scale,
                        style.use_lighter_font,
                        force_1px_height,
                    );
                } else {
                    renderer.block_render_char(
                        target,
                        dx as u32,
                        dy,
                        color,
                        style.foreground_alpha,
                        style.render_background,
                        style.background_alpha,
                        force_1px_height,
                    );
                }
                dx += i64::from(char_width);
                if dx > max_dx {
                    return;
                }
            }
        }
    }
}

/// Monaco `InnerMinimap.getXOffsetForPosition`: canvas x of 1-based
/// `column` on a line with UTF-16 `content`.
pub(crate) fn x_offset_for_position(
    content: &[u16],
    column: u32,
    tab_size: u32,
    char_width: f64,
    canvas_inner_width: f64,
) -> f64 {
    if column <= 1 {
        return MINIMAP_GUTTER_WIDTH;
    }
    let minimum_x_offset = f64::from(column - 1) * char_width;
    if minimum_x_offset >= canvas_inner_width {
        // Certainly past the minimap's width.
        return canvas_inner_width;
    }
    let mut x = MINIMAP_GUTTER_WIDTH;
    for (i, &code) in content.iter().enumerate() {
        if i + 1 >= column as usize {
            return x;
        }
        let dx = if code == u16::from(b'\t') {
            f64::from(tab_size) * char_width
        } else if crate::cursor::strings::is_full_width_character(u32::from(code)) {
            2. * char_width
        } else {
            char_width
        };
        x += dx;
        if x >= canvas_inner_width {
            return canvas_inner_width;
        }
    }
    if (column as usize - 1) <= content.len() {
        x
    } else {
        canvas_inner_width
    }
}

/// Monaco `InnerMinimap._fitSectionHeader`: `text` shortened in the middle
/// with an ellipsis to fit `max_width`, given its measured `width`, the
/// ellipsis's and its UTF-16 length.
pub(crate) fn fit_section_header(
    text: &str,
    width: f64,
    ellipsis_width: f64,
    max_width: f64,
) -> String {
    if text.is_empty() || width <= max_width || width <= ellipsis_width {
        return text.to_string();
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let len = units.len();
    let average_char_width = width / len as f64;
    let max_char_count = ((max_width - ellipsis_width) / average_char_width).floor() - 1.;
    let max_char_count = max_char_count.max(0.) as usize;
    // A halfway point that isn't after whitespace.
    let mut half_char_count = max_char_count.div_ceil(2).min(len);
    while half_char_count > 0
        && char::from_u32(u32::from(units[half_char_count - 1])).is_some_and(char::is_whitespace)
    {
        half_char_count -= 1;
    }
    let tail = (max_char_count - half_char_count.min(max_char_count)).min(len);
    let mut out = String::from_utf16_lossy(&units[..half_char_count]);
    out.push('…');
    out.push_str(&String::from_utf16_lossy(&units[len - tail..]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monaco `minimapCharRenderer.test.ts`: the letter `d`, as a canvas
    /// samples it.
    #[rustfmt::skip]
    const SAMPLE_D: [u8; 640] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xD0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xD0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xD0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x0D, 0xFF, 0xFF, 0xFF, 0xA3, 0xFF, 0xFF, 0xFF, 0xF3, 0xFF, 0xFF, 0xFF, 0xE5, 0xFF, 0xFF, 0xFF, 0x5E, 0xFF, 0xFF, 0xFF, 0xD0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF7, 0xFF, 0xFF, 0xFF, 0xFC, 0xFF, 0xFF, 0xFF, 0xF0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0x10, 0xFF, 0xFF, 0xFF, 0xFB, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x94, 0xFF, 0xFF, 0xFF, 0x02, 0xFF, 0xFF, 0xFF, 0x6A, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0x3B, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x22, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x03, 0xFF, 0xFF, 0xFF, 0xF0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0x47, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xD6, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0x31, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x16, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xE7, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0x0E, 0xFF, 0xFF, 0xFF, 0xF7, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x69, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x3D, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x9B, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF9, 0xFF, 0xFF, 0xFF, 0xB9, 0xFF, 0xFF, 0xFF, 0xF0, 0xFF, 0xFF, 0xFF, 0xF7, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x0E, 0xFF, 0xFF, 0xFF, 0xA7, 0xFF, 0xFF, 0xFF, 0xF5, 0xFF, 0xFF, 0xFF, 0xE8, 0xFF, 0xFF, 0xFF, 0x71, 0xFF, 0xFF, 0xFF, 0xD0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    /// Monaco's `getSampleData`: the full sheet with only `d` drawn.
    fn sample_data() -> Vec<u8> {
        let mut result = vec![
            0u8;
            sheet::SAMPLED_CHAR_HEIGHT
                * sheet::SAMPLED_CHAR_WIDTH
                * sheet::RGBA_CHANNELS_CNT
                * sheet::CHAR_COUNT as usize
        ];
        let row_width = sheet::RGBA_SAMPLED_ROW_WIDTH;
        let ch_index = (b'd' as i32 - sheet::START_CH_CODE) as usize;
        let cell = sheet::SAMPLED_CHAR_WIDTH * sheet::RGBA_CHANNELS_CNT;
        let mut global_output = ch_index * cell;
        for row in SAMPLE_D.chunks(cell) {
            result[global_output..global_output + cell].copy_from_slice(row);
            global_output += row_width;
        }
        result
    }

    fn render_d(scale: u32) -> Vec<u8> {
        let renderer = CharRenderer::from_sample_data(&sample_data(), scale);
        let background = Rgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        let color = Rgba8 {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        };
        let mut image = ImageData::filled(
            sheet::BASE_CHAR_WIDTH * scale,
            sheet::BASE_CHAR_HEIGHT * scale,
            background,
        );
        renderer.render_char(
            &mut image,
            0,
            0,
            u32::from(b'd'),
            color,
            255,
            background,
            255,
            scale,
            false,
            false,
        );
        image.data
    }

    #[test]
    fn letter_d_at_2x() {
        #[rustfmt::skip]
        assert_eq!(render_d(2), vec![
            0x2D, 0x2D, 0x2D, 0xFF, 0xAC, 0xAC, 0xAC, 0xFF,
            0xC6, 0xC6, 0xC6, 0xFF, 0xC8, 0xC8, 0xC8, 0xFF,
            0xC0, 0xC0, 0xC0, 0xFF, 0xCB, 0xCB, 0xCB, 0xFF,
            0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF,
        ]);
    }

    #[test]
    fn letter_d_at_1x() {
        #[rustfmt::skip]
        assert_eq!(render_d(1), vec![
            0xCB, 0xCB, 0xCB, 0xFF,
            0x81, 0x81, 0x81, 0xFF,
        ]);
    }

    #[test]
    fn prebaked_glyphs_cover_the_sheet() {
        assert_eq!(
            decode_data(PREBAKED_1).len(),
            sheet::CHAR_COUNT as usize * 2
        );
        assert_eq!(
            decode_data(PREBAKED_2).len(),
            sheet::CHAR_COUNT as usize * 8
        );
        // A space is blank; `#` isn't.
        let r = CharRenderer::create(2);
        assert!(r.normal[..8].iter().all(|&v| v == 0));
        let hash = char_index(u32::from(b'#'), 2) * 8;
        assert!(r.normal[hash..hash + 8].iter().any(|&v| v > 0));
        // Scale 3 is derived from the scale-2 glyphs.
        let r3 = CharRenderer::create(3);
        assert_eq!(r3.normal.len(), sheet::CHAR_COUNT as usize * 3 * 6);
        assert!(r3.normal[..18].iter().all(|&v| v == 0));
    }

    #[test]
    fn char_index_maps_unknown_characters() {
        assert_eq!(char_index(u32::from(b' '), 1), 0);
        assert_eq!(char_index(u32::from(b'~'), 1), 94);
        // Below the sheet: wrapped at small scales, else the unknown glyph.
        assert_eq!(char_index(10, 2), (10 - 32 + 96) as usize);
        assert_eq!(char_index(10, 3), 95);
        assert_eq!(char_index(0x4E2D, 3), 95);
    }

    fn options() -> LayoutOptions {
        LayoutOptions {
            pixel_ratio: 1.,
            minimap_line_height: 2.,
            canvas_inner_height: 400.,
            line_height: 20.,
            minimap_height_is_editor_height: false,
            scroll_beyond_last_line: true,
            padding_top: 0.,
            padding_bottom: 0.,
            minimap_height: 400.,
        }
    }

    fn input(line_count: u32, scroll_top: f64) -> LayoutInput {
        let start = (scroll_top / 20.).floor() as u32 + 1;
        // Scroll beyond the last line: the editor's height less a line.
        let scroll_height = f64::from(line_count) * 20. + 380.;
        LayoutInput {
            viewport_start_line_number: start,
            viewport_end_line_number: (start + 20).min(line_count),
            viewport_start_line_number_vertical_offset: f64::from(start - 1) * 20.,
            viewport_height: 400.,
            viewport_contains_whitespace_gaps: false,
            line_count,
            real_line_count: line_count,
            scroll_top,
            scroll_height,
        }
    }

    #[test]
    fn layout_when_all_lines_fit() {
        let layout = MinimapLayout::create(&options(), &input(100, 0.), None);
        assert_eq!((layout.start_line_number, layout.end_line_number), (1, 100));
        // 20 visible lines at 2px each.
        assert_eq!(layout.slider_height, 40.);
        assert_eq!(layout.slider_top, 0.);
        assert!(layout.slider_needed);
        // Scrolled to the end, the last line is at the slider's top.
        let max = 100. * 20. + 380. - 400.;
        let end = MinimapLayout::create(&options(), &input(100, max), None);
        assert_eq!(end.slider_top, 196.);
        assert_eq!(end.y_for_line_number(100, 2.), 198.);
        // Dragging by the slider's travel scrolls to the end.
        assert_eq!(layout.desired_scroll_top_from_delta(end.slider_top), max);
    }

    #[test]
    fn layout_of_a_long_file_shows_the_end() {
        // 1000 lines don't fit 200 minimap lines.
        let max = 1000. * 20. + 380. - 400.;
        let end = MinimapLayout::create(&options(), &input(1000, max), None);
        assert_eq!(end.end_line_number, 1000);
        // The slider stays inside the minimap with the last line at its top.
        assert!(end.slider_top + end.slider_height <= 400.);
        assert_eq!(
            end.slider_top,
            end.y_for_line_number(1000, 2.),
            "the last line is at the slider's top"
        );
        // Halfway, the slider is about halfway down.
        let mid = MinimapLayout::create(&options(), &input(1000, max / 2.), None);
        assert!((mid.slider_top - (400. - 40.) / 2.).abs() < 2.);
        let first_visible = mid.y_for_line_number(mid.scroll_top as u32 / 20 + 1, 2.);
        assert!((mid.slider_top - first_visible).abs() < 2.);
    }

    #[test]
    fn layout_keeps_the_start_line_while_scrolling() {
        let a = MinimapLayout::create(&options(), &input(1000, 5000.), None);
        let b = MinimapLayout::create(&options(), &input(1000, 5010.), Some(&a));
        assert!(b.start_line_number >= a.start_line_number);
        let c = MinimapLayout::create(&options(), &input(1000, 5000.), Some(&b));
        assert!(c.start_line_number <= b.start_line_number);
    }

    #[test]
    fn layout_filling_the_editor() {
        let options = LayoutOptions {
            minimap_height_is_editor_height: true,
            ..options()
        };
        let max = 1000. * 20. + 380. - 400.;
        let layout = MinimapLayout::create(&options, &input(1000, max), None);
        // 400 * 400 / 20380.
        assert_eq!(layout.slider_height, 7.);
        assert!((layout.slider_top - (400. - 7.)).abs() < 1e-9);
        assert_eq!(layout.start_line_number, 1);
    }

    #[test]
    fn sampling() {
        let state = SamplingState::compute(1000, 100.);
        assert_eq!(state.minimap_lines.len(), 100);
        assert_eq!(state.minimap_lines[0], 1);
        assert_eq!(state.minimap_lines[1], 15);
        assert_eq!(state.minimap_lines[99], 1000);
        assert_eq!(state.model_line_to_minimap_line(500), 50);
        assert_eq!(
            state.decoration_line_range_to_minimap_line_range(500, 501),
            (50, 51)
        );
        assert_eq!(
            state.decoration_line_range_to_minimap_line_range(999, 1000),
            (99, 100)
        );
    }

    #[test]
    fn x_offsets() {
        let content: Vec<u16> = "\tab中c".encode_utf16().collect();
        let x = |column| x_offset_for_position(&content, column, 4, 2., 100.);
        assert_eq!(x(1), 8.);
        assert_eq!(x(2), 16.);
        assert_eq!(x(4), 20.);
        assert_eq!(x(5), 24.);
        assert_eq!(x(6), 26.);
        assert_eq!(x(60), 100.);
    }

    #[test]
    fn render_line_blends_glyphs_after_the_gutter() {
        let background = Rgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        let style = LineStyle {
            text: true,
            font_scale: 1,
            char_width: 1,
            minimap_line_height: 2.,
            render_background: background,
            background_alpha: 255,
            foreground_alpha: 255,
            use_lighter_font: false,
            tab_size: 4,
        };
        let mut image = ImageData::filled(16, 2, background);
        let content: Vec<u16> = "\t#".encode_utf16().collect();
        let white = Rgba8 {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        };
        render_line(
            &mut image,
            &CharRenderer::create(1),
            &style,
            0.,
            &content,
            |_| white,
        );
        // The tab takes four cells after the 8px gutter, then `#`.
        let lit: Vec<usize> = (0..16)
            .filter(|&x| image.data[x * 4] > 0 || image.data[(16 + x) * 4] > 0)
            .collect();
        assert_eq!(lit, vec![12]);
    }

    #[test]
    fn section_header_fitting() {
        // 20 characters 2px each into 21px with a 4px ellipsis.
        let text = "abcdefghij klmnopqrs";
        assert_eq!(fit_section_header(text, 40., 4., 50.), text);
        let fitted = fit_section_header(text, 40., 4., 21.);
        // floor(17 / 2) - 1 = 7 characters: 4 before, 3 after.
        assert_eq!(fitted, "abcd…qrs");
    }
}
