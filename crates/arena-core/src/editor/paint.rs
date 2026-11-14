//! Saved chassis paint stays in the native 32-bit TGA payload.
use crate::{Vec2, robot::Chassis};
pub fn solid_tga(width: u16, height: u16, color: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[2] = 2;
    bytes[12..14].copy_from_slice(&width.to_le_bytes());
    bytes[14..16].copy_from_slice(&height.to_le_bytes());
    bytes[16] = 32;
    for _ in 0..width as usize * height as usize {
        bytes.extend([color[2], color[1], color[0], color[3]]);
    }
    bytes
}
pub fn rgba(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    if bytes.len() < 18 || bytes[2] != 2 || bytes[16] != 32 {
        return Err("Expected raw 32-bit TGA".into());
    }
    let w = u16::from_le_bytes([bytes[12], bytes[13]]) as usize;
    let h = u16::from_le_bytes([bytes[14], bytes[15]]) as usize;
    if bytes.len() != 18 + w * h * 4 {
        return Err("Truncated TGA pixels".into());
    }
    let mut out = vec![0; w * h * 4];
    for y in 0..h {
        let row = if bytes[17] & 32 == 0 { h - y - 1 } else { y };
        for x in 0..w {
            let p = 18 + (row * w + x) * 4;
            let o = (y * w + x) * 4;
            out[o..o + 4].copy_from_slice(&[bytes[p + 2], bytes[p + 1], bytes[p], 255]);
        }
    }
    Ok((w, h, out))
}
pub fn from_rgba(width: u16, height: u16, pixels: &[u8]) -> Result<Vec<u8>, String> {
    if pixels.len() != width as usize * height as usize * 4 {
        return Err("RGBA image size mismatch".into());
    }
    let mut bytes = solid_tga(width, height, [0; 4]);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let p = (y * width as usize + x) * 4;
            let o = 18 + ((height as usize - y - 1) * width as usize + x) * 4;
            bytes[o..o + 4].copy_from_slice(&[pixels[p + 2], pixels[p + 1], pixels[p], 0]);
        }
    }
    Ok(bytes)
}
fn inside(p: Vec2, polygon: &[Vec2]) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}
pub fn fill(chassis: &mut Chassis, panel: Option<usize>, color: [u8; 4]) -> Result<(), String> {
    let bytes = chassis.texture.as_mut().ok_or("Chassis has no paint map")?;
    for y in 0..256 {
        for x in 0..256 {
            let selected = panel.is_none_or(|id| {
                chassis
                    .paint_layout
                    .group_uvs
                    .get(id)
                    .is_some_and(|p| inside(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), p))
            });
            if selected {
                let offset = 18 + ((255 - y) * 256 + x) * 4;
                bytes[offset..offset + 4].copy_from_slice(&[color[2], color[1], color[0], 0]);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub enum TextureEffect {
    Fill,
    Layer,
    Edge { width: f32, length_ratio: f32 },
    Decal { center: Vec2, size: f32 },
}
/// Composite a source image into native paint groups; padding and unrelated panels survive.
pub fn texture(
    chassis: &mut Chassis,
    panel: Option<usize>,
    source: (usize, usize, &[u8]),
    effect: TextureEffect,
) -> Result<(), String> {
    let (sw, sh, rgba) = source;
    if sw == 0 || sh == 0 || rgba.len() != sw * sh * 4 {
        return Err("Invalid paint source".into());
    }
    let old = chassis.texture.as_deref().ok_or("Build a chassis first")?;
    let (w, h, mut out) = self::rgba(old)?;
    for (id, poly) in chassis.paint_layout.group_uvs.iter().enumerate() {
        if panel.is_some_and(|p| p != id) || poly.len() < 3 {
            continue;
        }
        let min = poly.iter().copied().reduce(Vec2::min).unwrap();
        let max = poly.iter().copied().reduce(Vec2::max).unwrap();
        for y in 0..h {
            for x in 0..w {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if !inside(p, poly) {
                    continue;
                }
                let uv = match effect {
                    TextureEffect::Fill | TextureEffect::Layer => {
                        (p - min) / (max - min).max(Vec2::ONE)
                    }
                    TextureEffect::Decal { center, size } => {
                        let uv = (p - center) / size.max(1.0) + Vec2::splat(0.5);
                        if uv.min_element() < 0.0 || uv.max_element() >= 1.0 {
                            continue;
                        }
                        uv
                    }
                    TextureEffect::Edge {
                        width,
                        length_ratio,
                    } => {
                        let mut best = (f32::INFINITY, 0.0);
                        let mut length = 0.0;
                        for i in 0..poly.len() {
                            let a = poly[i];
                            let b = poly[(i + 1) % poly.len()];
                            let e = b - a;
                            let l = e.length();
                            let t =
                                ((p - a).dot(e) / e.length_squared().max(0.0001)).clamp(0.0, 1.0);
                            let d = p.distance(a + e * t);
                            if d < best.0 {
                                best = (d, length + t * l);
                            }
                            length += l;
                        }
                        if best.0 >= width.max(1.0) {
                            continue;
                        }
                        Vec2::new(
                            (best.1 / (sw as f32 * length_ratio.max(0.001))).fract(),
                            best.0 / width.max(1.0),
                        )
                    }
                };
                let sx = (uv.x * sw as f32).floor().max(0.0) as usize % sw;
                let sy = (uv.y * sh as f32).floor().max(0.0) as usize % sh;
                let s = (sy * sw + sx) * 4;
                let o = (y * w + x) * 4;
                let alpha = if matches!(effect, TextureEffect::Fill) {
                    1.0
                } else {
                    rgba[s + 3] as f32 / 255.0
                };
                for c in 0..3 {
                    out[o + c] = (rgba[s + c] as f32 * alpha + out[o + c] as f32 * (1.0 - alpha))
                        .round() as u8;
                }
            }
        }
    }
    chassis.texture = Some(from_rgba(w as u16, h as u16, &out)?);
    Ok(())
}
/// Uncompressed 24-bit Windows BMP, the original custom-texture exchange format.
pub fn bmp(width: usize, height: usize, pixels: &[u8]) -> Result<Vec<u8>, String> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || pixels.len() != width * height * 4
    {
        return Err("Invalid BMP dimensions".into());
    }
    let stride = (width * 3 + 3) & !3;
    let mut out = vec![0; 54 + stride * height];
    out[..2].copy_from_slice(b"BM");
    let size = out.len() as u32;
    out[2..6].copy_from_slice(&size.to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    out[22..26].copy_from_slice(&(height as i32).to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..height {
        for x in 0..width {
            let p = (y * width + x) * 4;
            let o = 54 + (height - y - 1) * stride + x * 3;
            out[o..o + 3].copy_from_slice(&[pixels[p + 2], pixels[p + 1], pixels[p]]);
        }
    }
    Ok(out)
}
