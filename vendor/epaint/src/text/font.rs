use std::collections::BTreeMap;
use std::sync::Arc;

use emath::{vec2, Vec2};

use crate::{
    mutex::{Mutex, RwLock},
    text::FontTweak,
    TextureAtlas,
};

// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct UvRect {
    /// X/Y offset for nice rendering (unit: points).
    pub offset: Vec2,

    /// Screen size (in points) of this glyph.
    /// Note that the height is different from the font height.
    pub size: Vec2,

    /// Top left corner UV in texture.
    pub min: [u16; 2],

    /// Bottom right corner (exclusive).
    pub max: [u16; 2],
}

impl UvRect {
    pub fn is_nothing(&self) -> bool {
        self.min == self.max
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphInfo {
    /// Used for pair-kerning.
    ///
    /// Doesn't need to be unique.
    /// Use `ab_glyph::GlyphId(0)` if you just want to have an id, and don't care.
    pub(crate) id: ab_glyph::GlyphId,

    /// Unit: points.
    pub advance_width: f32,

    /// Texture coordinates.
    pub uv_rect: UvRect,
}

impl Default for GlyphInfo {
    /// Basically a zero-width space.
    fn default() -> Self {
        Self {
            id: ab_glyph::GlyphId(0),
            advance_width: 0.0,
            uv_rect: Default::default(),
        }
    }
}

// ----------------------------------------------------------------------------

static SUBPIXEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Draw letters on the red, green and blue parts of each pixel (as ClearType does):
/// 0 = off (grey edges), 1 = screens with R,G,B left to right, 2 = B,G,R.
/// Set it before any text is drawn; letters already drawn keep their kind.
pub fn set_subpixel(order: u8) {
    SUBPIXEL.store(order.min(2), std::sync::atomic::Ordering::Relaxed);
}

/// A glyph drawn for subpixels takes three texels per pixel, and its texel x is moved right by
/// twice the atlas width, so the painter sees u >= 2 and knows to read three texels per pixel
/// (see egui_glow's fragment shader)
pub const SUBPIXEL_U_SHIFT: f32 = 2.0;

/// FreeType's default LCD filter: spreads each subpixel a little onto its neighbours, so
/// edges don't show colour fringes
const LCD_FILTER: [u32; 5] = [0x08, 0x4D, 0x56, 0x4D, 0x08];

/// The font file's bytes and its index in the file, for drawing hinted letters
pub(crate) type RawFont = (Arc<[u8]>, u32);

/// Collects a hinted outline as a path for the rasterizer, flipped so y points down
/// (the second field stretches x: 3 for subpixel drawing)
struct Pen(Vec<zeno::Command>, f32);

impl skrifa::outline::OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(zeno::Command::MoveTo([x * self.1, -y].into()));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(zeno::Command::LineTo([x * self.1, -y].into()));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.push(zeno::Command::QuadTo([cx * self.1, -cy].into(), [x * self.1, -y].into()));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let k = self.1;
        self.0.push(zeno::Command::CurveTo([cx0 * k, -cy0].into(), [cx1 * k, -cy1].into(), [x * k, -y].into()));
    }
    fn close(&mut self) {
        self.0.push(zeno::Command::Close);
    }
}

/// How strongly partly covered pixels are lit (1 = as covered). A little under 1 keeps the
/// weight of light text on dark close to egui's, with the edges much less smeared than its 0.55
const TEXT_GAMMA: f32 = 0.8;

/// A specific font with a size.
/// The interface uses points as the unit for everything.
pub struct FontImpl {
    name: String,
    ab_glyph_font: ab_glyph::FontArc,
    raw: RawFont,
    hinter: std::sync::OnceLock<Option<skrifa::outline::HintingInstance>>,

    /// Maximum character height
    scale_in_pixels: u32,

    height_in_points: f32,

    // move each character by this much (hack)
    y_offset_in_points: f32,

    ascent: f32,
    pixels_per_point: f32,
    glyph_info_cache: RwLock<ahash::HashMap<char, GlyphInfo>>, // TODO(emilk): standard Mutex
    atlas: Arc<Mutex<TextureAtlas>>,
}

impl FontImpl {
    pub fn new(
        atlas: Arc<Mutex<TextureAtlas>>,
        pixels_per_point: f32,
        name: String,
        ab_glyph_font: ab_glyph::FontArc,
        scale_in_pixels: f32,
        tweak: FontTweak,
        raw: RawFont,
    ) -> Self {
        assert!(scale_in_pixels > 0.0);
        assert!(pixels_per_point > 0.0);

        use ab_glyph::{Font, ScaleFont};
        let scaled = ab_glyph_font.as_scaled(scale_in_pixels);
        let ascent = scaled.ascent() / pixels_per_point;
        let descent = scaled.descent() / pixels_per_point;
        let line_gap = scaled.line_gap() / pixels_per_point;

        // Tweak the scale as the user desired
        let scale_in_pixels = scale_in_pixels * tweak.scale;

        let baseline_offset = {
            let scale_in_points = scale_in_pixels / pixels_per_point;
            scale_in_points * tweak.baseline_offset_factor
        };

        let y_offset_points = {
            let scale_in_points = scale_in_pixels / pixels_per_point;
            scale_in_points * tweak.y_offset_factor
        } + tweak.y_offset;

        // Center scaled glyphs properly:
        let height = ascent + descent;
        let y_offset_points = y_offset_points - (1.0 - tweak.scale) * 0.5 * height;

        // Round to an even number of physical pixels to get even kerning.
        // See https://github.com/emilk/egui/issues/382
        let scale_in_pixels = scale_in_pixels.round() as u32;

        // Round to closest pixel:
        let y_offset_in_points = (y_offset_points * pixels_per_point).round() / pixels_per_point;

        Self {
            name,
            ab_glyph_font,
            raw,
            hinter: Default::default(),
            scale_in_pixels,
            height_in_points: ascent - descent + line_gap,
            y_offset_in_points,
            ascent: ascent + baseline_offset,
            pixels_per_point,
            glyph_info_cache: Default::default(),
            atlas,
        }
    }

    /// Code points that will always be replaced by the replacement character.
    ///
    /// See also [`invisible_char`].
    fn ignore_character(&self, chr: char) -> bool {
        use crate::text::FontDefinitions;

        if !FontDefinitions::builtin_font_names().contains(&self.name.as_str()) {
            return false;
        }

        if self.name == "emoji-icon-font" {
            // HACK: https://github.com/emilk/egui/issues/1284 https://github.com/jslegers/emoji-icon-font/issues/18
            // Don't show the wrong fullwidth capital letters:
            if 'Ｓ' <= chr && chr <= 'Ｙ' {
                return true;
            }
        }

        matches!(
            chr,
            // Strip out a religious symbol with secondary nefarious interpretation:
            '\u{534d}' | '\u{5350}' |

            // Ignore ubuntu-specific stuff in `Ubuntu-Light.ttf`:
            '\u{E0FF}' | '\u{EFFD}' | '\u{F0FF}' | '\u{F200}'
        )
    }

    /// An un-ordered iterator over all supported characters.
    fn characters(&self) -> impl Iterator<Item = char> + '_ {
        use ab_glyph::Font as _;
        self.ab_glyph_font
            .codepoint_ids()
            .map(|(_, chr)| chr)
            .filter(|&chr| !self.ignore_character(chr))
    }

    /// `\n` will result in `None`
    fn glyph_info(&self, c: char) -> Option<GlyphInfo> {
        {
            if let Some(glyph_info) = self.glyph_info_cache.read().get(&c) {
                return Some(*glyph_info);
            }
        }

        if self.ignore_character(c) {
            return None; // these will result in the replacement character when rendering
        }

        if c == '\t' {
            if let Some(space) = self.glyph_info(' ') {
                let glyph_info = GlyphInfo {
                    advance_width: crate::text::TAB_SIZE as f32 * space.advance_width,
                    ..space
                };
                self.glyph_info_cache.write().insert(c, glyph_info);
                return Some(glyph_info);
            }
        }

        if c == '\u{2009}' {
            // Thin space, often used as thousands deliminator: 1 234 567 890
            // https://www.compart.com/en/unicode/U+2009
            // https://en.wikipedia.org/wiki/Thin_space

            if let Some(space) = self.glyph_info(' ') {
                let em = self.height_in_points; // TODO(emilk): is this right?
                let advance_width = f32::min(em / 6.0, space.advance_width * 0.5);
                let glyph_info = GlyphInfo {
                    advance_width,
                    ..space
                };
                self.glyph_info_cache.write().insert(c, glyph_info);
                return Some(glyph_info);
            }
        }

        if invisible_char(c) {
            let glyph_info = GlyphInfo::default();
            self.glyph_info_cache.write().insert(c, glyph_info);
            return Some(glyph_info);
        }

        // Add new character:
        use ab_glyph::Font as _;
        let glyph_id = self.ab_glyph_font.glyph_id(c);

        if glyph_id.0 == 0 {
            None // unsupported character
        } else {
            let glyph_info = self.allocate_glyph(glyph_id);
            self.glyph_info_cache.write().insert(c, glyph_info);
            Some(glyph_info)
        }
    }

    #[inline]
    pub fn pair_kerning(
        &self,
        last_glyph_id: ab_glyph::GlyphId,
        glyph_id: ab_glyph::GlyphId,
    ) -> f32 {
        use ab_glyph::{Font as _, ScaleFont};
        self.ab_glyph_font
            .as_scaled(self.scale_in_pixels as f32)
            .kern(last_glyph_id, glyph_id)
            / self.pixels_per_point
    }

    /// Height of one row of text in points.
    #[inline(always)]
    pub fn row_height(&self) -> f32 {
        self.height_in_points
    }

    #[inline(always)]
    pub fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    /// This is the distance from the top to the baseline.
    ///
    /// Unit: points.
    #[inline(always)]
    pub fn ascent(&self) -> f32 {
        self.ascent
    }

    /// Draws the letter with the font's own hinting, so its stems and edges land on whole pixels
    /// (as Windows draws text) instead of being smeared over two. `None` falls back to egui's own
    /// drawing; `Some(default)` is a letter with nothing to draw (a space)
    fn hinted_glyph(&self, glyph_id: ab_glyph::GlyphId) -> Option<UvRect> {
        use skrifa::outline::DrawSettings;
        use skrifa::MetadataProvider as _;
        let hinter = self.hinter.get_or_init(|| self.make_hinter()).as_ref()?;
        let font = skrifa::FontRef::from_index(&self.raw.0, self.raw.1).ok()?;
        let glyph = font.outline_glyphs().get(skrifa::GlyphId::new(glyph_id.0 as _))?;
        let order = SUBPIXEL.load(std::sync::atomic::Ordering::Relaxed);
        let mut pen = Pen(Vec::new(), if order == 0 { 1.0 } else { 3.0 });
        glyph.draw(DrawSettings::hinted(hinter, false), &mut pen).ok()?;
        if pen.0.is_empty() {
            return Some(UvRect::default());
        }
        let (data, place) = zeno::Mask::new(&pen.0).format(zeno::Format::Alpha).render();
        if order != 0 {
            return self.subpixel_glyph(&data, place, order);
        }
        let (w, h) = (place.width as usize, place.height as usize);
        if w == 0 || h == 0 {
            return Some(UvRect::default());
        }
        if data.len() < w * h {
            return None;
        }
        // the atlas lifts every pixel to the power 0.55 when it is uploaded; undo that here
        let gamma = TEXT_GAMMA;
        let glyph_pos = {
            let atlas = &mut self.atlas.lock();
            let (glyph_pos, image) = atlas.allocate((w, h));
            for y in 0..h {
                for x in 0..w {
                    let v = data[y * w + x];
                    if v != 0 {
                        image[(glyph_pos.0 + x, glyph_pos.1 + y)] = (v as f32 / 255.0).powf(gamma / 0.55);
                    }
                }
            }
            glyph_pos
        };
        let offset_in_pixels = vec2(place.left as f32, place.top as f32);
        Some(UvRect {
            offset: offset_in_pixels / self.pixels_per_point + self.y_offset_in_points * Vec2::Y,
            size: vec2(w as f32, h as f32) / self.pixels_per_point,
            min: [glyph_pos.0 as u16, glyph_pos.1 as u16],
            max: [(glyph_pos.0 + w) as u16, (glyph_pos.1 + h) as u16],
        })
    }

    /// Places a glyph drawn three times as wide: filters it, and stores three texels per pixel
    fn subpixel_glyph(&self, data: &[u8], place: zeno::Placement, order: u8) -> Option<UvRect> {
        let (ws, h) = (place.width as i32, place.height as usize);
        if ws == 0 || h == 0 {
            return Some(UvRect::default());
        }
        if data.len() < ws as usize * h {
            return None;
        }
        // whole pixels covering the glyph plus the filter's reach of two subpixels each side
        let px0 = (place.left - 2).div_euclid(3);
        let px1 = (place.left + ws + 2 + 2).div_euclid(3);
        let w = (px1 - px0) as usize;
        let tw = w * 3;
        let gamma = TEXT_GAMMA;
        let atlas_w = self.atlas.lock().size()[0];
        let glyph_pos = {
            let atlas = &mut self.atlas.lock();
            let (glyph_pos, image) = atlas.allocate((tw, h));
            for y in 0..h {
                let row = &data[y * ws as usize..(y + 1) * ws as usize];
                let raw = |s: i32| -> u32 {
                    let i = s - place.left;
                    if (0..ws).contains(&i) { row[i as usize] as u32 } else { 0 }
                };
                for t in 0..tw as i32 {
                    let s = px0 * 3 + t;
                    let v: u32 = (0..5).map(|k| LCD_FILTER[k as usize] * raw(s - 2 + k)).sum::<u32>() / 256;
                    if v == 0 {
                        continue;
                    }
                    // texels are always R, G, B; on a B,G,R screen the left subpixel is blue
                    let tt = if order == 2 { t - t % 3 + (2 - t % 3) } else { t } as usize;
                    // the atlas lifts every pixel to the power 0.55 when it is uploaded; undo that here
                    image[(glyph_pos.0 + tt, glyph_pos.1 + y)] = (v.min(255) as f32 / 255.0).powf(gamma / 0.55);
                }
            }
            glyph_pos
        };
        let shift = (SUBPIXEL_U_SHIFT as usize) * atlas_w;
        let offset_in_pixels = vec2(px0 as f32, place.top as f32);
        Some(UvRect {
            offset: offset_in_pixels / self.pixels_per_point + self.y_offset_in_points * Vec2::Y,
            size: vec2(w as f32, h as f32) / self.pixels_per_point,
            min: [(shift + glyph_pos.0) as u16, glyph_pos.1 as u16],
            max: [(shift + glyph_pos.0 + tw) as u16, (glyph_pos.1 + h) as u16],
        })
    }

    /// Runs the font's hinting program for this size, once
    fn make_hinter(&self) -> Option<skrifa::outline::HintingInstance> {
        use ab_glyph::Font as _;
        use skrifa::instance::{LocationRef, Size};
        use skrifa::outline::{HintingInstance, HintingMode};
        use skrifa::MetadataProvider as _;
        // the hinting meant for smoothed text (FreeType's "normal"); the strong kind, meant for
        // black-and-white text, thins semibold down to regular
        let mode = HintingMode::Smooth { lcd_subpixel: None, preserve_linear_metrics: false };
        let font = skrifa::FontRef::from_index(&self.raw.0, self.raw.1).ok()?;
        // egui's scale is the font's full height in pixels; hinting wants pixels per em
        let upem = self.ab_glyph_font.units_per_em()?;
        let ppem = self.scale_in_pixels as f32 * upem / self.ab_glyph_font.height_unscaled();
        HintingInstance::new(&font.outline_glyphs(), Size::new(ppem), LocationRef::default(), mode).ok()
    }

    fn allocate_glyph(&self, glyph_id: ab_glyph::GlyphId) -> GlyphInfo {
        assert!(glyph_id.0 != 0);
        use ab_glyph::{Font as _, ScaleFont};

        let glyph = glyph_id.with_scale_and_position(
            self.scale_in_pixels as f32,
            ab_glyph::Point { x: 0.0, y: 0.0 },
        );

        let uv_rect = match self.hinted_glyph(glyph_id) {
            Some(r) => Some(r),
            None => self.ab_glyph_font.outline_glyph(glyph).map(|glyph| {
            let bb = glyph.px_bounds();
            let glyph_width = bb.width() as usize;
            let glyph_height = bb.height() as usize;
            if glyph_width == 0 || glyph_height == 0 {
                UvRect::default()
            } else {
                let glyph_pos = {
                    let atlas = &mut self.atlas.lock();
                    let (glyph_pos, image) = atlas.allocate((glyph_width, glyph_height));
                    glyph.draw(|x, y, v| {
                        if 0.0 < v {
                            let px = glyph_pos.0 + x as usize;
                            let py = glyph_pos.1 + y as usize;
                            image[(px, py)] = v;
                        }
                    });
                    glyph_pos
                };

                let offset_in_pixels = vec2(bb.min.x, bb.min.y);
                let offset =
                    offset_in_pixels / self.pixels_per_point + self.y_offset_in_points * Vec2::Y;
                UvRect {
                    offset,
                    size: vec2(glyph_width as f32, glyph_height as f32) / self.pixels_per_point,
                    min: [glyph_pos.0 as u16, glyph_pos.1 as u16],
                    max: [
                        (glyph_pos.0 + glyph_width) as u16,
                        (glyph_pos.1 + glyph_height) as u16,
                    ],
                }
            }
        }),
        };
        let uv_rect = uv_rect.unwrap_or_default();

        let advance_width_in_points = self
            .ab_glyph_font
            .as_scaled(self.scale_in_pixels as f32)
            .h_advance(glyph_id)
            / self.pixels_per_point;

        GlyphInfo {
            id: glyph_id,
            advance_width: advance_width_in_points,
            uv_rect,
        }
    }
}

type FontIndex = usize;

// TODO(emilk): rename?
/// Wrapper over multiple [`FontImpl`] (e.g. a primary + fallbacks for emojis)
pub struct Font {
    fonts: Vec<Arc<FontImpl>>,

    /// Lazily calculated.
    characters: Option<BTreeMap<char, Vec<String>>>,

    replacement_glyph: (FontIndex, GlyphInfo),
    pixels_per_point: f32,
    row_height: f32,
    glyph_info_cache: ahash::HashMap<char, (FontIndex, GlyphInfo)>,
}

impl Font {
    pub fn new(fonts: Vec<Arc<FontImpl>>) -> Self {
        if fonts.is_empty() {
            return Self {
                fonts,
                characters: None,
                replacement_glyph: Default::default(),
                pixels_per_point: 1.0,
                row_height: 0.0,
                glyph_info_cache: Default::default(),
            };
        }

        let pixels_per_point = fonts[0].pixels_per_point();
        let row_height = fonts[0].row_height();

        let mut slf = Self {
            fonts,
            characters: None,
            replacement_glyph: Default::default(),
            pixels_per_point,
            row_height,
            glyph_info_cache: Default::default(),
        };

        const PRIMARY_REPLACEMENT_CHAR: char = '◻'; // white medium square
        const FALLBACK_REPLACEMENT_CHAR: char = '?'; // fallback for the fallback

        let replacement_glyph = slf
            .glyph_info_no_cache_or_fallback(PRIMARY_REPLACEMENT_CHAR)
            .or_else(|| slf.glyph_info_no_cache_or_fallback(FALLBACK_REPLACEMENT_CHAR))
            .unwrap_or_else(|| {
                #[cfg(feature = "log")]
                log::warn!(
                    "Failed to find replacement characters {PRIMARY_REPLACEMENT_CHAR:?} or {FALLBACK_REPLACEMENT_CHAR:?}. Will use empty glyph."
                );
                (0, GlyphInfo::default())
            });
        slf.replacement_glyph = replacement_glyph;

        slf
    }

    pub fn preload_characters(&mut self, s: &str) {
        for c in s.chars() {
            self.glyph_info(c);
        }
    }

    pub fn preload_common_characters(&mut self) {
        // Preload the printable ASCII characters [32, 126] (which excludes control codes):
        const FIRST_ASCII: usize = 32; // 32 == space
        const LAST_ASCII: usize = 126;
        for c in (FIRST_ASCII..=LAST_ASCII).map(|c| c as u8 as char) {
            self.glyph_info(c);
        }
        self.glyph_info('°');
        self.glyph_info(crate::text::PASSWORD_REPLACEMENT_CHAR);
    }

    /// All supported characters, and in which font they are available in.
    pub fn characters(&mut self) -> &BTreeMap<char, Vec<String>> {
        self.characters.get_or_insert_with(|| {
            let mut characters: BTreeMap<char, Vec<String>> = Default::default();
            for font in &self.fonts {
                for chr in font.characters() {
                    characters.entry(chr).or_default().push(font.name.clone());
                }
            }
            characters
        })
    }

    #[inline(always)]
    pub fn round_to_pixel(&self, point: f32) -> f32 {
        (point * self.pixels_per_point).round() / self.pixels_per_point
    }

    /// Height of one row of text. In points
    #[inline(always)]
    pub fn row_height(&self) -> f32 {
        self.row_height
    }

    pub fn uv_rect(&self, c: char) -> UvRect {
        self.glyph_info_cache
            .get(&c)
            .map(|gi| gi.1.uv_rect)
            .unwrap_or_default()
    }

    /// Width of this character in points.
    pub fn glyph_width(&mut self, c: char) -> f32 {
        self.glyph_info(c).1.advance_width
    }

    /// Can we display this glyph?
    pub fn has_glyph(&mut self, c: char) -> bool {
        self.glyph_info(c) != self.replacement_glyph // TODO(emilk): this is a false negative if the user asks about the replacement character itself 🤦‍♂️
    }

    /// Can we display all the glyphs in this text?
    pub fn has_glyphs(&mut self, s: &str) -> bool {
        s.chars().all(|c| self.has_glyph(c))
    }

    /// `\n` will (intentionally) show up as the replacement character.
    fn glyph_info(&mut self, c: char) -> (FontIndex, GlyphInfo) {
        if let Some(font_index_glyph_info) = self.glyph_info_cache.get(&c) {
            return *font_index_glyph_info;
        }

        let font_index_glyph_info = self.glyph_info_no_cache_or_fallback(c);
        let font_index_glyph_info = font_index_glyph_info.unwrap_or(self.replacement_glyph);
        self.glyph_info_cache.insert(c, font_index_glyph_info);
        font_index_glyph_info
    }

    #[inline]
    pub(crate) fn font_impl_and_glyph_info(&mut self, c: char) -> (Option<&FontImpl>, GlyphInfo) {
        if self.fonts.is_empty() {
            return (None, self.replacement_glyph.1);
        }
        let (font_index, glyph_info) = self.glyph_info(c);
        let font_impl = &self.fonts[font_index];
        (Some(font_impl), glyph_info)
    }

    pub(crate) fn ascent(&self) -> f32 {
        if let Some(first) = self.fonts.first() {
            first.ascent()
        } else {
            self.row_height
        }
    }

    fn glyph_info_no_cache_or_fallback(&mut self, c: char) -> Option<(FontIndex, GlyphInfo)> {
        for (font_index, font_impl) in self.fonts.iter().enumerate() {
            if let Some(glyph_info) = font_impl.glyph_info(c) {
                self.glyph_info_cache.insert(c, (font_index, glyph_info));
                return Some((font_index, glyph_info));
            }
        }
        None
    }
}

/// Code points that will always be invisible (zero width).
///
/// See also [`FontImpl::ignore_character`].
#[inline]
fn invisible_char(c: char) -> bool {
    if c == '\r' {
        // A character most vile and pernicious. Don't display it.
        return true;
    }

    // See https://github.com/emilk/egui/issues/336

    // From https://www.fileformat.info/info/unicode/category/Cf/list.htm

    // TODO(emilk): heed bidi characters

    matches!(
        c,
        '\u{200B}' // ZERO WIDTH SPACE
            | '\u{200C}' // ZERO WIDTH NON-JOINER
            | '\u{200D}' // ZERO WIDTH JOINER
            | '\u{200E}' // LEFT-TO-RIGHT MARK
            | '\u{200F}' // RIGHT-TO-LEFT MARK
            | '\u{202A}' // LEFT-TO-RIGHT EMBEDDING
            | '\u{202B}' // RIGHT-TO-LEFT EMBEDDING
            | '\u{202C}' // POP DIRECTIONAL FORMATTING
            | '\u{202D}' // LEFT-TO-RIGHT OVERRIDE
            | '\u{202E}' // RIGHT-TO-LEFT OVERRIDE
            | '\u{2060}' // WORD JOINER
            | '\u{2061}' // FUNCTION APPLICATION
            | '\u{2062}' // INVISIBLE TIMES
            | '\u{2063}' // INVISIBLE SEPARATOR
            | '\u{2064}' // INVISIBLE PLUS
            | '\u{2066}' // LEFT-TO-RIGHT ISOLATE
            | '\u{2067}' // RIGHT-TO-LEFT ISOLATE
            | '\u{2068}' // FIRST STRONG ISOLATE
            | '\u{2069}' // POP DIRECTIONAL ISOLATE
            | '\u{206A}' // INHIBIT SYMMETRIC SWAPPING
            | '\u{206B}' // ACTIVATE SYMMETRIC SWAPPING
            | '\u{206C}' // INHIBIT ARABIC FORM SHAPING
            | '\u{206D}' // ACTIVATE ARABIC FORM SHAPING
            | '\u{206E}' // NATIONAL DIGIT SHAPES
            | '\u{206F}' // NOMINAL DIGIT SHAPES
            | '\u{FEFF}' // ZERO WIDTH NO-BREAK SPACE
    )
}
