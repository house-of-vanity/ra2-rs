//! Persistent UV paint-loss mask; original BOT/TGA data remains untouched.
use super::*;
use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub(super) struct Paint {
    pub width: u32,
    pub height: u32,
    original: Vec<u8>,
    mask: Vec<f32>,
    pub revision: u64,
}

impl Paint {
    pub fn new(bytes: Option<&[u8]>) -> Result<Self, String> {
        let decoded = bytes
            .map(|bytes| {
                Image::from_buffer(
                    bytes,
                    ImageType::Extension("tga"),
                    CompressedImageFormats::NONE,
                    true,
                    ImageSampler::linear(),
                    RenderAssetUsages::default(),
                )
                .map_err(|e| format!("Cannot decode chassis paint: {e}"))?
                .convert(TextureFormat::Rgba8UnormSrgb)
                .ok_or_else(|| "Unsupported chassis paint format".to_string())
            })
            .transpose()?;
        let (sw, sh, source) = decoded
            .and_then(|image| {
                let size = image.texture_descriptor.size;
                image.data.map(|bytes| (size.width, size.height, bytes))
            })
            .unwrap_or((1, 1, vec![180, 180, 180, 255]));
        let width = sw.clamp(512, 2048);
        let height = sh.clamp(512, 2048);
        let mut original = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let i = ((y * sh / height * sw + x * sw / width) * 4) as usize;
                original.extend_from_slice(&source[i..i + 3]);
                original.push(255);
            }
        }
        Ok(Self {
            width,
            height,
            original,
            mask: vec![0.0; (width * height) as usize],
            revision: 0,
        })
    }

    pub fn sample(&self, uv: Vec2) -> [f32; 4] {
        let x = (uv.x.clamp(0.0, 1.0) * (self.width - 1) as f32) as u32;
        let y = (uv.y.clamp(0.0, 1.0) * (self.height - 1) as f32) as u32;
        let i = ((y * self.width + x) * 4) as usize;
        Color::srgb_u8(self.original[i], self.original[i + 1], self.original[i + 2])
            .to_linear()
            .to_f32_array()
    }

    /// Rasterize the *original*, sparse surface in UV space. Brush size and cost
    /// do not depend on the deformation mesh density; the marks follow its UVs.
    pub fn strike(
        &mut self,
        model: &arena_core::Model,
        center: Vec3,
        outward: Vec3,
        radius: f32,
        power: f32,
        seed: u32,
    ) {
        let radius = radius.max(0.005);
        let axis = Quat::from_rotation_arc(Vec3::Y, outward);
        let right = axis * Vec3::X;
        let up = axis * Vec3::Z;
        let angle = seed as f32 * 2.3999631;
        let (sin, cos) = angle.sin_cos();
        let mut changed = false;
        for object in model.objects.iter().filter(|o| o.visible) {
            for face in &object.faces {
                let p = face
                    .vertices
                    .map(|i| object.transform.transform_point3(object.vertices[i]));
                let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
                if n.dot(outward) < 0.5 {
                    continue;
                }
                let min = p[0].min(p[1]).min(p[2]);
                let max = p[0].max(p[1]).max(p[2]);
                if center.distance_squared(center.clamp(min, max)) > radius * radius * 2.25 {
                    continue;
                }
                let uv = face.uv.map(Vec2::from_array);
                let lo = uv[0].min(uv[1]).min(uv[2]).clamp(Vec2::ZERO, Vec2::ONE);
                let hi = uv[0].max(uv[1]).max(uv[2]).clamp(Vec2::ZERO, Vec2::ONE);
                let a = uv[1] - uv[0];
                let b = uv[2] - uv[0];
                let determinant = a.perp_dot(b);
                if determinant.abs() < 1e-10 {
                    continue;
                }
                let x0 = (lo.x * self.width as f32).floor() as u32;
                let x1 = ((hi.x * self.width as f32).ceil() as u32).min(self.width - 1);
                let y0 = (lo.y * self.height as f32).floor() as u32;
                let y1 = ((hi.y * self.height as f32).ceil() as u32).min(self.height - 1);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let point = Vec2::new(
                            (x as f32 + 0.5) / self.width as f32,
                            (y as f32 + 0.5) / self.height as f32,
                        );
                        let offset = point - uv[0];
                        let v = offset.perp_dot(b) / determinant;
                        let w = a.perp_dot(offset) / determinant;
                        if v < -0.00001 || w < -0.00001 || v + w > 1.00001 {
                            continue;
                        }
                        let delta = p[0] * (1.0 - v - w) + p[1] * v + p[2] * w - center;
                        let u = delta.dot(right) / radius;
                        let v = delta.dot(up) / radius;
                        let u2 = u * cos - v * sin;
                        let v2 = u * sin + v * cos;
                        let grain = noise(
                            (delta.dot(right) * 300.0).floor() as i32,
                            (delta.dot(up) * 300.0).floor() as i32,
                            seed,
                        );
                        let distance = (u * u + v * v).sqrt();
                        let chip = ((0.85 + grain * 0.3 - distance) * 7.0).clamp(0.0, 1.0)
                            * (0.5 + grain * 0.5);
                        let scratch = (0..3)
                            .map(|i| {
                                let line = (v2 - (i as f32 - 1.0) * 0.17).abs();
                                ((0.027 - line) * 65.0).clamp(0.0, 1.0)
                                    * ((1.35 - u2.abs()) * 5.0).clamp(0.0, 1.0)
                            })
                            .fold(0.0_f32, f32::max);
                        let amount = (chip.max(scratch) * power * 0.7).clamp(0.0, 1.0);
                        let i = (y * self.width + x) as usize;
                        let next = 1.0 - (1.0 - self.mask[i]) * (1.0 - amount);
                        if next > self.mask[i] + 0.0001 {
                            self.mask[i] = next;
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.revision += 1;
        }
    }

    pub fn textures(&self, strength: f32) -> (Vec<u8>, Vec<u8>) {
        let mut color = self.original.clone();
        let mut surface = Vec::with_capacity(color.len());
        for (i, p) in color.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let mask = (self.mask[i] * strength).clamp(0.0, 1.0);
            let x = i % self.width as usize;
            let y = i / self.width as usize;
            let metal =
                135.0 + noise(x as i32, y as i32, 713) * 38.0 + (y as f32 * 0.7).sin() * 7.0;
            for c in &mut p[..3] {
                *c = (*c as f32 * (1.0 - mask) + metal * mask).round() as u8;
            }
            surface.extend_from_slice(&[
                255,
                (140.0 - 60.0 * mask) as u8,
                (230.0 * mask) as u8,
                255,
            ]);
        }
        (color, surface)
    }
    pub fn image(&self, bytes: Vec<u8>, srgb: bool) -> Image {
        let mut image = Image::new(
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            bytes,
            if srgb {
                TextureFormat::Rgba8UnormSrgb
            } else {
                TextureFormat::Rgba8Unorm
            },
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::linear();
        image
    }
    #[cfg(test)]
    pub fn covered(&self) -> usize {
        self.mask.iter().filter(|m| **m > 0.001).count()
    }
}
fn noise(x: i32, y: i32, seed: u32) -> f32 {
    let mut v = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263) ^ seed;
    v = (v ^ (v >> 13)).wrapping_mul(1274126177);
    (v ^ (v >> 16)) as f32 / u32::MAX as f32
}

pub(super) fn hit_uv(shell: &arena_core::damage::DentMesh, hit: SurfaceHit) -> Vec2 {
    let t = &shell.triangles[hit.triangle];
    let p = t.nodes.map(|i| shell.rest[i]);
    let a = p[1] - p[0];
    let b = p[2] - p[0];
    let q = hit.rest_point - p[0];
    let aa = a.dot(a);
    let ab = a.dot(b);
    let bb = b.dot(b);
    let d = aa * bb - ab * ab;
    if d.abs() < 1e-12 {
        return t.uv[0];
    }
    let v = (bb * q.dot(a) - ab * q.dot(b)) / d;
    let w = (aa * q.dot(b) - ab * q.dot(a)) / d;
    t.uv[0] * (1.0 - v - w) + t.uv[1] * v + t.uv[2] * w
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wear_mask_is_local_persistent_and_can_reveal_original_paint() {
        use arena_core::{Face, Model, Object, damage::DentMesh};
        let model = Model {
            objects: vec![Object {
                name: "panel".into(),
                visible: true,
                parent: None,
                transform: Mat4::IDENTITY,
                vertices: vec![
                    Vec3::new(-1., 0., -1.),
                    Vec3::new(-1., 0., 1.),
                    Vec3::new(1., 0., 1.),
                    Vec3::new(1., 0., -1.),
                ],
                faces: vec![
                    Face {
                        vertices: [0, 1, 2],
                        uv: [[0., 0.], [0., 1.], [1., 1.]],
                        normals: [Vec3::Y; 3],
                        material: 0,
                    },
                    Face {
                        vertices: [0, 2, 3],
                        uv: [[0., 0.], [1., 1.], [1., 0.]],
                        normals: [Vec3::Y; 3],
                        material: 0,
                    },
                ],
                material: 0,
                uv_channels: default(),
            }],
            ..default()
        };
        let shell = DentMesh::from_model(&model, 0.35).unwrap();
        let hit = shell.raycast(Vec3::Y, -Vec3::Y).unwrap();
        assert!(hit_uv(&shell, hit).abs_diff_eq(Vec2::splat(0.5), 0.00001));
        let mut paint = Paint::new(None).unwrap();
        for p in paint.original.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&[210, 30, 20, 255]);
        }
        let source = paint.original.clone();
        let sampled = paint.sample(Vec2::splat(0.5));
        paint.strike(&model, hit.rest_point, Vec3::Y, 0.12, 1.0, 7);
        assert!(paint.covered() > 0 && paint.covered() < paint.mask.len() / 10);
        let (worn, surface) = paint.textures(1.0);
        assert_ne!(worn, source);
        assert!(surface.as_chunks::<4>().0.iter().any(|p| p[2] > 0));
        assert_eq!(paint.textures(0.0).0, source);
        assert_eq!(
            paint.sample(Vec2::splat(0.5)),
            sampled,
            "Particles must sample paint, not bare metal"
        );
        assert_eq!(paint.original, source);
        let revision = paint.revision;
        let covered = paint.covered();
        paint.strike(&model, hit.rest_point, -Vec3::Y, 0.12, 1.0, 7);
        assert_eq!(
            paint.covered(),
            covered,
            "Back face brush leaked through the panel"
        );
        assert_eq!(paint.revision, revision);
    }
}
