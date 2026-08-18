use chrono::Local;
use defy::Contextualize;
use fs_err as fs;
use graphics::character::{Character, CharacterCache};
use graphics::glyph_cache::rusttype::GlyphCache;
use graphics::types::FontSize;
use once_cell::sync::Lazy;
use opengl_graphics::{Texture, TextureSettings};
use rusttype::{Font, GlyphId, Scale};

use crate::error::Error;

pub const FONT_SIZE: u32 = 11;
const FONT_SCALE: Scale = Scale { x: 15.0, y: 15.0 };

fn get_font_ref() -> &'static Font<'static> {
    const FONT_DATA: &[u8] = include_bytes!("../assets/Inconsolata-Regular.ttf");
    static FONT: Lazy<Font<'static>> =
        Lazy::new(|| Font::try_from_bytes(FONT_DATA).expect("unable to load font"));

    &*FONT
}

/// Locates readable system fallback fonts on this machine. The platform layer
/// supplies Windows and Linux candidates; missing files remain best-effort.
fn load_system_fallback_fonts() -> Vec<Font<'static>> {
    load_fonts_from_candidates(crate::platform::fonts::system_font_candidates())
}

fn load_fonts_from_candidates(
    candidates: impl IntoIterator<Item = crate::platform::fonts::SystemFontCandidate>,
) -> Vec<Font<'static>> {
    let mut fonts = Vec::new();
    for candidate in candidates {
        let path = candidate.path;
        let Ok(bytes) = fs::read(&path) else { continue };
        match Font::try_from_vec_and_index(bytes, candidate.index) {
            Some(font) => fonts.push(font),
            None => eprintln!("Fallback font at {} could not be parsed", path.display()),
        }
    }
    fonts
}

/// A [`CharacterCache`] that renders text with a primary font, drawing any
/// glyph the primary font is missing from a chain of system fallback fonts
/// instead of the primary font's `.notdef` box ("tofu").
pub struct MultiFontGlyphCache<'a> {
    caches: Vec<GlyphCache<'a, (), Texture>>,
}

impl std::fmt::Debug for MultiFontGlyphCache<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiFontGlyphCache")
            .field("fonts", &self.caches.len())
            .finish()
    }
}

impl<'a> MultiFontGlyphCache<'a> {
    fn new(fonts: Vec<Font<'a>>, settings: TextureSettings) -> Self {
        let caches = fonts
            .into_iter()
            .map(|font| GlyphCache::from_font(font, (), settings))
            .collect();
        MultiFontGlyphCache { caches }
    }

    pub fn preload_printable_ascii(&mut self, size: FontSize) {
        self.caches[0]
            .preload_printable_ascii(size)
            .expect("unable to preload font glyphs");
    }

    fn cache_index_for(&self, ch: char) -> usize {
        self.caches
            .iter()
            .position(|cache| cache.font.glyph(ch).id() != GlyphId(0))
            .unwrap_or(0)
    }
}

impl<'a> CharacterCache for MultiFontGlyphCache<'a> {
    type Texture = Texture;
    type Error = <GlyphCache<'a, (), Texture> as CharacterCache>::Error;

    fn character(
        &mut self,
        size: FontSize,
        ch: char,
    ) -> Result<Character<'_, Texture>, Self::Error> {
        let index = self.cache_index_for(ch);
        self.caches[index].character(size, ch)
    }
}

/// Builds the glyph cache used for all UI text: the embedded Inconsolata
/// font plus whichever platform system fonts are available locally for
/// scripts it does not cover (Cyrillic, Simplified Chinese).
pub fn get_glyph_cache(settings: TextureSettings) -> MultiFontGlyphCache<'static> {
    let mut fonts = vec![get_font_ref().clone()];
    fonts.extend(load_system_fallback_fonts());
    MultiFontGlyphCache::new(fonts, settings)
}

pub fn get_width_metric(ch: char) -> f64 {
    get_font_ref()
        .glyph(ch)
        .scaled(FONT_SCALE)
        .h_metrics()
        .advance_width as f64
}

pub fn get_width_metric_str(s: &str) -> f64 {
    get_font_ref()
        .glyphs_for(s.chars())
        .map(|glyph| glyph.scaled(FONT_SCALE).h_metrics().advance_width)
        .sum::<f32>() as f64
}

pub fn get_height_metric() -> f64 {
    let v_metrics = get_font_ref().v_metrics(FONT_SCALE);
    (v_metrics.ascent - v_metrics.descent) as f64
}

pub fn get_v_metrics() -> VMetrics {
    let v_metrics = get_font_ref().v_metrics(FONT_SCALE);
    VMetrics {
        ascent: v_metrics.ascent as f64,
        descent: v_metrics.descent as f64,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VMetrics {
    pub ascent: f64,
    pub descent: f64,
}

pub fn view_font_license() -> Result<(), Error> {
    const LICENSE_CONTENTS: &[u8] = include_bytes!("../assets/Inconsolata-OFL.txt");

    let now = Local::now().format("%Y%m%d-%H%M%S");
    let directory = crate::platform::AppPaths::from_process()
        .temporary_dir()
        .map_err(|error| Error::from(error.to_string()))?;
    fs::create_dir_all(&directory).context("Failed to create font license directory")?;
    let path = directory.join(format!("Inconsolata-OFL-{}.txt", now));

    fs::write(&path, LICENSE_CONTENTS).context("Failed to write font license to disk")?;

    crate::platform::desktop::open_font_license(&path)
        .map_err(|error| Error::from(format!("Failed to open license: {error}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_font_covers_accented_latin_directly() {
        let cache = get_glyph_cache(TextureSettings::new());
        for ch in "áàâãéêíóôõúçàâæçéèêëîïôœùûüÿáéíóúüñ".chars() {
            assert_eq!(
                cache.cache_index_for(ch),
                0,
                "'{ch}' should not need a fallback font"
            );
        }
    }

    #[test]
    fn system_font_loading_is_best_effort() {
        let cache = get_glyph_cache(TextureSettings::new());
        // Installed system font packages vary by distribution and desktop image.
        // Candidate selection is tested separately without depending on this machine.
        assert!(!cache.caches.is_empty());
    }

    #[test]
    fn missing_system_font_candidates_are_skipped() {
        let fonts = load_fonts_from_candidates([crate::platform::fonts::SystemFontCandidate {
            path: std::path::PathBuf::from("definitely-missing-font.ttf"),
            index: 0,
        }]);
        assert!(fonts.is_empty());
    }
}
