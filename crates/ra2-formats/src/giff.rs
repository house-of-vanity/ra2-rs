//! Original GIFF bitmap fonts: ASCII metrics and an embedded ARGB4444 TGA atlas.
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Glyph {
    pub uv: [f32; 4],
    pub advance: i32,
    pub left: i32,
    pub top: i32,
}
#[derive(Clone, Debug)]
pub struct BitmapFont {
    pub line_height: i32,
    pub baseline: i32,
    /// Visible ink above and below the baseline, excluding transparent atlas padding.
    pub ascent: i32,
    pub descent: i32,
    pub space: i32,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub glyphs: BTreeMap<char, Glyph>,
}
impl BitmapFont {
    pub fn glyph(&self, ch: char) -> Option<&Glyph> {
        self.glyphs.get(&ch).or_else(|| self.glyphs.get(&'?'))
    }

    pub fn advance(&self, ch: char) -> f32 {
        self.glyph(ch).map_or(0, |g| g.advance) as f32
    }

    pub fn measure(&self, text: &str) -> f32 {
        text.chars().map(|c| self.advance(c)).sum()
    }
}
fn word(bytes: &[u8], offset: usize) -> Result<[u8; 4], String> {
    bytes
        .get(offset..offset + 4)
        .ok_or("Truncated GIFF")
        .map(|s| s.try_into().unwrap())
        .map_err(String::from)
}
pub fn decode(bytes: &[u8]) -> Result<BitmapFont, String> {
    const HEADER: usize = 24;
    const COUNT: usize = 95;
    const RECORD: usize = 28;
    let integer = |offset| Ok::<i32, String>(i32::from_le_bytes(word(bytes, offset)?));
    let line_height = integer(0)?;
    let baseline = integer(4)?.checked_neg().ok_or("Invalid GIFF baseline")?;
    let space = integer(8)?;
    let width = integer(16)?;
    let height = integer(20)?;
    if !(1..=256).contains(&line_height)
        || !(1..=4096).contains(&width)
        || !(1..=4096).contains(&height)
    {
        return Err("Invalid GIFF dimensions".into());
    }
    let mut glyphs = BTreeMap::new();
    for i in 0..COUNT {
        let offset = HEADER + i * RECORD;
        let mut uv = [0.0; 4];
        for (j, value) in uv.iter_mut().enumerate() {
            *value = f32::from_le_bytes(word(bytes, offset + j * 4)?);
        }
        if uv
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("Invalid GIFF glyph UV".into());
        }
        let top = integer(offset + 24)?;
        if !(-4096..=4096).contains(&top) {
            return Err("Invalid GIFF glyph bearing".into());
        }
        glyphs.insert(
            char::from(i as u8 + 32),
            Glyph {
                uv,
                advance: integer(offset + 16)?,
                left: integer(offset + 20)?,
                top,
            },
        );
    }
    let tga = HEADER + COUNT * RECORD;
    let header = bytes.get(tga..tga + 18).ok_or("Missing GIFF atlas")?;
    if header[0] != 0
        || header[2] != 2
        || header[16] != 16
        || header[17] & 15 != 4
        || u16::from_le_bytes([header[12], header[13]]) as i32 != width
        || u16::from_le_bytes([header[14], header[15]]) as i32 != height
    {
        return Err("Unsupported GIFF atlas layout".into());
    }
    let (width, height) = (width as usize, height as usize);
    let data = bytes
        .get(tga + 18..tga + 18 + width * height * 2)
        .ok_or("Truncated GIFF atlas pixels")?;
    let mut rgba = vec![0; width * height * 4];
    for y in 0..height {
        let source_y = if header[17] & 32 == 0 {
            height - y - 1
        } else {
            y
        };
        for x in 0..width {
            let at = (source_y * width + x) * 2;
            let p = u16::from_le_bytes([data[at], data[at + 1]]);
            let out = (y * width + x) * 4;
            rgba[out..out + 4].copy_from_slice(&[
                ((p >> 8) & 15) as u8 * 17,
                ((p >> 4) & 15) as u8 * 17,
                (p & 15) as u8 * 17,
                ((p >> 12) & 15) as u8 * 17,
            ]);
        }
    }
    let mut ascent = 0;
    let mut descent = 0;
    for g in glyphs.values() {
        let x0 = (g.uv[0] * width as f32).round() as usize;
        let x1 = (g.uv[2] * width as f32).round() as usize;
        let y0 = (g.uv[1] * height as f32).round() as usize;
        let y1 = (g.uv[3] * height as f32).round() as usize;
        for y in y0..y1 {
            if (x0..x1).any(|x| rgba[(y * width + x) * 4 + 3] != 0) {
                let row = (y - y0) as i32 - g.top;
                ascent = ascent.max(-row);
                descent = descent.max(row + 1);
            }
        }
    }
    Ok(BitmapFont {
        line_height,
        baseline,
        ascent,
        descent,
        space,
        width,
        height,
        rgba,
        glyphs,
    })
}
