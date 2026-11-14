//! Small deterministic 2D compositor for the original GUI artwork and bitmap fonts.
use bevy::prelude::*;
use ra2_formats::giff::BitmapFont;

#[derive(Clone)]
pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}
impl Pixels {
    pub fn blank(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; width * height * 4],
        }
    }
    pub fn bounds(&self) -> Rect {
        Rect::from_corners(Vec2::ZERO, Vec2::new(self.width as f32, self.height as f32))
    }
    fn pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let at = (y as usize * self.width + x as usize) * 4;
        let sa = color[3] as f32 / 255.0;
        if sa == 0.0 {
            return;
        }
        let da = self.rgba[at + 3] as f32 / 255.0;
        let alpha = sa + da * (1.0 - sa);
        for (i, &source) in color.iter().enumerate().take(3) {
            self.rgba[at + i] = ((source as f32 * sa + self.rgba[at + i] as f32 * da * (1.0 - sa))
                / alpha)
                .round() as u8;
        }
        self.rgba[at + 3] = (alpha * 255.0).round() as u8;
    }
    pub fn clear(&mut self, rect: Rect) {
        let r = rect.intersect(self.bounds());
        for y in r.min.y.ceil() as usize..r.max.y.max(0.0) as usize {
            for x in r.min.x.ceil() as usize..r.max.x.max(0.0) as usize {
                self.rgba[(y * self.width + x) * 4..(y * self.width + x) * 4 + 4].fill(0);
            }
        }
    }
    pub fn fill(&mut self, rect: Rect, color: [f32; 4], clip: Rect) {
        let r = rect.intersect(clip).intersect(self.bounds());
        let color = color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        // Cover pixel centres consistently. Percent-based widget positions are
        // fractional; ceil(min)..floor(max) erased one-pixel bottom/right edges.
        for y in (r.min.y - 0.5).ceil() as i32..(r.max.y - 0.5).ceil() as i32 {
            for x in (r.min.x - 0.5).ceil() as i32..(r.max.x - 0.5).ceil() as i32 {
                self.pixel(x, y, color);
            }
        }
    }
    pub fn border(&mut self, rect: Rect, width: f32, color: [f32; 4], clip: Rect) {
        self.fill(
            Rect::from_corners(rect.min, Vec2::new(rect.max.x, rect.min.y + width)),
            color,
            clip,
        );
        self.fill(
            Rect::from_corners(Vec2::new(rect.min.x, rect.max.y - width), rect.max),
            color,
            clip,
        );
        self.fill(
            Rect::from_corners(rect.min, Vec2::new(rect.min.x + width, rect.max.y)),
            color,
            clip,
        );
        self.fill(
            Rect::from_corners(Vec2::new(rect.max.x - width, rect.min.y), rect.max),
            color,
            clip,
        );
    }
    pub fn line(&mut self, a: Vec2, b: Vec2, width: f32, color: [f32; 4], clip: Rect) {
        let steps = a.distance(b).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let p = a.lerp(b, i as f32 / steps as f32);
            let min = (p - Vec2::splat(width * 0.5)).floor();
            self.fill(
                Rect::from_corners(min, min + Vec2::splat(width.ceil().max(1.0))),
                color,
                clip,
            );
        }
    }
    pub fn image(&mut self, source: &Pixels, rect: Rect, tint: [f32; 4], alpha: bool, clip: Rect) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let r = rect.intersect(clip).intersect(self.bounds());
        for y in r.min.y.ceil() as i32..r.max.y as i32 {
            for x in r.min.x.ceil() as i32..r.max.x as i32 {
                let sx = (((x as f32 - rect.min.x) / rect.width() * source.width as f32) as usize)
                    .min(source.width - 1);
                let sy = (((y as f32 - rect.min.y) / rect.height() * source.height as f32)
                    as usize)
                    .min(source.height - 1);
                let at = (sy * source.width + sx) * 4;
                let mut color = [0; 4];
                for i in 0..4 {
                    color[i] = (source.rgba[at + i] as f32 * tint[i]).clamp(0.0, 255.0) as u8;
                }
                if !alpha {
                    color[3] = (255.0 * tint[3]) as u8;
                }
                self.pixel(x, y, color);
            }
        }
    }
    pub fn text(
        &mut self,
        font: &BitmapFont,
        text: &str,
        origin: Vec2,
        color: [f32; 4],
        clip: Rect,
        wrap: bool,
    ) -> Vec2 {
        let mut pen = origin;
        let mut max_x = origin.x;
        for word in text.replace("\\n", "\n").split_inclusive([' ', '\n']) {
            let advance = word
                .chars()
                .filter_map(|c| font.glyphs.get(&c))
                .map(|g| g.advance)
                .sum::<i32>() as f32;
            if wrap && pen.x > origin.x && pen.x + advance > clip.max.x {
                pen.x = origin.x;
                pen.y += font.line_height as f32;
            }
            for ch in word.chars() {
                if ch == '\n' {
                    pen.x = origin.x;
                    pen.y += font.line_height as f32;
                    continue;
                }
                let Some(g) = font.glyphs.get(&ch).or_else(|| font.glyphs.get(&'?')) else {
                    continue;
                };
                let x0 = (g.uv[0] * font.width as f32).round() as usize;
                let y0 = (g.uv[1] * font.height as f32).round() as usize;
                let x1 = (g.uv[2] * font.width as f32).round() as usize;
                let y1 = (g.uv[3] * font.height as f32).round() as usize;
                for sy in y0..y1.min(font.height) {
                    for sx in x0..x1.min(font.width) {
                        let x = pen.x + g.left as f32 + (sx - x0) as f32;
                        let y = pen.y - g.top as f32 + (sy - y0) as f32;
                        if !clip.contains(Vec2::new(x, y)) {
                            continue;
                        }
                        let at = (sy * font.width + sx) * 4;
                        let mut pixel = [0; 4];
                        for i in 0..4 {
                            pixel[i] =
                                (font.rgba[at + i] as f32 * color[i]).clamp(0.0, 255.0) as u8;
                        }
                        self.pixel(x.round() as i32, y.round() as i32, pixel);
                    }
                }
                pen.x += g.advance as f32;
                max_x = max_x.max(pen.x);
            }
        }
        Vec2::new(max_x - origin.x, pen.y - origin.y + font.line_height as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_widget_bounds_keep_all_four_single_pixel_borders() {
        let mut pixels = Pixels::blank(16, 16);
        let rect = Rect::from_corners(Vec2::new(2.2, 2.2), Vec2::new(12.2, 12.2));
        pixels.border(rect, 1., [1.; 4], pixels.bounds());
        let opaque = |x: usize, y: usize| pixels.rgba[(y * 16 + x) * 4 + 3] == 255;
        for at in 2..12 {
            assert!(opaque(at, 2) && opaque(at, 11) && opaque(2, at) && opaque(11, at));
        }
        assert!(!opaque(7, 7));
    }
}
