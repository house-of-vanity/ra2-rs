//! Fill gaps in the original game fonts so player names and chat can use other alphabets.
use ab_glyph::{Font, FontRef, ScaleFont};
use ra2_formats::giff::{BitmapFont, Glyph};

pub(super) fn fallback_font() -> FontRef<'static> {
    FontRef::try_from_slice(include_bytes!("../../assets/fonts/Ubuntu-Light.ttf"))
        .expect("bundled Ubuntu font")
}

/// Append rows to the existing atlas. All menus then use the same glyph metrics for
/// rendering, wrapping and hit testing, including when another peer supplied the text.
pub(super) fn extend(font: &mut BitmapFont, fallback: &FontRef<'_>) {
    let old_height = font.height;
    let scale = (font.ascent + font.descent).max(1) as f32;
    let scaled = fallback.as_scaled(scale);
    let (mut x, mut y, mut row_height) = (0usize, old_height, 0usize);
    let mut extra = Vec::new();
    for (id, ch) in fallback.codepoint_ids() {
        if ch.is_ascii() || ch.is_control() || font.glyphs.contains_key(&ch) {
            continue;
        }
        let mut glyph = Glyph {
            uv: [0.; 4],
            advance: scaled.h_advance(id).round() as i32,
            left: 0,
            top: 0,
        };
        let Some(outline) = fallback.outline_glyph(id.with_scale(scale)) else {
            extra.push((ch, glyph));
            continue;
        };
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width() as usize, bounds.height() as usize);
        if w > font.width {
            continue;
        }
        if x + w > font.width {
            x = 0;
            y += row_height;
            row_height = 0;
        }
        font.height = font.height.max(y + h);
        font.rgba.resize(font.width * font.height * 4, 0);
        outline.draw(|col, row, coverage| {
            let i = ((y + row as usize) * font.width + x + col as usize) * 4;
            font.rgba[i..i + 4].copy_from_slice(&[255, 255, 255, (coverage * 255.).round() as u8]);
        });
        glyph.left = bounds.min.x as i32;
        glyph.top = -bounds.min.y as i32;
        // Store vertical coordinates in pixels until the final height is known.
        glyph.uv = [
            x as f32 / font.width as f32,
            y as f32,
            (x + w) as f32 / font.width as f32,
            (y + h) as f32,
        ];
        extra.push((ch, glyph));
        x += w;
        row_height = row_height.max(h);
    }
    let original_scale = old_height as f32 / font.height as f32;
    for glyph in font.glyphs.values_mut() {
        glyph.uv[1] *= original_scale;
        glyph.uv[3] *= original_scale;
    }
    for (ch, mut glyph) in extra {
        glyph.uv[1] /= font.height as f32;
        glyph.uv[3] /= font.height as f32;
        font.glyphs.insert(ch, glyph);
    }
}
