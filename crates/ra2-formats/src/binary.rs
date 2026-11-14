//! GMI v3. Known records are consumed by schema, not by their unreliable length fields.
//! Unknown record types fail with a byte offset; we never guess where a next record begins.
use crate::{
    ImportReport, ImportedModel, SourceExtension, geometry, properties, reflect, right_handed,
};
use arena_core::*;
use std::collections::BTreeMap;

const MAX_COUNT: usize = 2_000_000;
const MAX_DEPTH: usize = 64;

struct Chunk {
    id: u32,
    offset: usize,
    start: usize,
    length: usize,
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    context: String,
    report: ImportReport,
}
type Result<T> = std::result::Result<T, String>;

impl Reader<'_> {
    fn error(&self, message: impl std::fmt::Display) -> String {
        format!("GMI at 0x{:x} ({}): {message}", self.pos, self.context)
    }
    fn take(&mut self, count: usize) -> Result<&[u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| self.error("Size overflow"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| self.error(format!("Truncated file: need {count} bytes")))?;
        self.pos = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if !v.is_finite() {
            return Err(self.error("Non-finite float"));
        }
        Ok(v)
    }
    fn flag(&mut self) -> Result<bool> {
        Ok(self.take(1)?[0] != 0)
    }
    fn count(&mut self, minimum_bytes: usize) -> Result<usize> {
        let count = self.u32()? as usize;
        if count > MAX_COUNT || count.saturating_mul(minimum_bytes) > self.bytes.len() - self.pos {
            return Err(self.error(format!("Invalid or oversized count: {count}")));
        }
        Ok(count)
    }
    fn string(&mut self) -> Result<String> {
        let length = self.count(1)?;
        let bytes = self.take(length)?;
        if bytes.is_empty() {
            return Ok(String::new());
        }
        if bytes.last() != Some(&0) {
            return Err(self.error("Unterminated string"));
        }
        // File names and node names are normally ASCII; Windows-1252 byte values are
        // retained losslessly as Unicode scalars instead of rejecting localized files.
        Ok(bytes[..bytes.len() - 1]
            .iter()
            .map(|&b| char::from(b))
            .collect())
    }
    fn vector(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn uv(&mut self) -> Result<Vec2> {
        let u = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        // The exported W component is unused for 2D textures; several shipped files
        // contain 0xffffffff (NaN) here. Do not let this poison usable U/V data.
        self.take(4)?;
        if !u.is_finite() || !v.is_finite() {
            self.report.warnings.push(format!(
                "Invalid texture coordinate at 0x{:x}; replaced non-finite U/V with zero",
                self.pos - 12
            ));
        }
        Ok(Vec2::new(
            if u.is_finite() { u } else { 0.0 },
            1.0 - if v.is_finite() { v } else { 0.0 },
        ))
    }
    fn indices(&mut self, limit: usize) -> Result<[usize; 3]> {
        let ids = [
            self.u32()? as usize,
            self.u32()? as usize,
            self.u32()? as usize,
        ];
        if ids.iter().any(|i| *i >= limit) {
            return Err(self.error("Index outside vertex buffer"));
        }
        Ok(ids)
    }
    fn chunk(&mut self, expected: u32) -> Result<Chunk> {
        let offset = self.pos;
        let id = self.u32()?;
        let flags = self.u32()?;
        let length = self.u32()? as usize;
        if id != expected || !(2..=4).contains(&flags) {
            return Err(self.error(format!(
                "Expected record {expected}, got {id} (flags {flags})"
            )));
        }
        Ok(Chunk {
            id,
            offset,
            start: self.pos,
            length,
        })
    }
    fn finish(&mut self, chunk: &Chunk) {
        let actual = self.pos - chunk.start;
        if actual != chunk.length {
            self.report.warnings.push(format!(
                "Record {} at 0x{:x}: declared {} bytes, consumed {actual} by schema",
                chunk.id, chunk.offset, chunk.length
            ));
        }
    }
    fn extension(&mut self, chunk: &Chunk) {
        self.report.extensions.push(SourceExtension {
            node_type: chunk.id,
            offset: chunk.offset,
            bytes: self.bytes[chunk.offset..self.pos].to_vec(),
        });
    }
    fn tm(&mut self) -> Result<Mat4> {
        let c = self.chunk(17)?;
        let _name = self.string()?;
        let m = Mat4::from_cols(
            self.vector()?.extend(0.0),
            self.vector()?.extend(0.0),
            self.vector()?.extend(0.0),
            self.vector()?.extend(1.0),
        );
        self.finish(&c);
        Ok(right_handed(m))
    }
    fn material_list(&mut self, depth: usize) -> Result<BTreeMap<usize, Material>> {
        if depth > MAX_DEPTH {
            return Err(self.error("Material nesting too deep"));
        }
        let chunk = self.chunk(7)?;
        let count = self.count(12)?;
        let mut materials = BTreeMap::new();
        for _ in 0..count {
            let c = self.chunk(8)?;
            let id = self.u32()? as usize;
            let name = self.string()?;
            let _class = self.string()?;
            self.take(4)?; // ambient
            let rgb = self.take(4)?;
            let color = [
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
            ];
            self.take(4)?; // specular
            self.f32()?;
            self.f32()?; // shine, strength
            let alpha = 1.0 - self.f32()?.clamp(0.0, 1.0);
            self.f32()?;
            self.u32()?; // wire, shading
            self.f32()?;
            self.f32()?; // falloff, self illumination
            let two_sided = self.u32()? != 0;
            self.u32()?; // transparency type
            let texture_count = self.count(0)?;
            let textures = if texture_count > 0 {
                self.textures(depth + 1)?
            } else {
                vec![]
            };
            if textures.len() != texture_count {
                return Err(self.error("Texture count mismatch"));
            }
            let sub_count = self.count(0)?;
            let submaterials = if sub_count > 0 {
                self.material_list(depth + 1)?
            } else {
                BTreeMap::new()
            };
            if submaterials.len() != sub_count {
                return Err(self.error("Submaterial count mismatch"));
            }
            let texture = textures
                .iter()
                .find(|(kind, _)| *kind == 1)
                .map(|(_, t)| t.clone());
            let opacity_texture = textures
                .iter()
                .find(|(kind, _)| *kind == 6)
                .map(|(_, t)| t.clone());
            if materials
                .insert(
                    id,
                    Material {
                        name,
                        color,
                        alpha,
                        two_sided,
                        texture,
                        opacity_texture,
                        submaterials,
                    },
                )
                .is_some()
            {
                return Err(self.error("Duplicate material identifier"));
            }
            self.finish(&c);
        }
        self.finish(&chunk);
        Ok(materials)
    }
    fn textures(&mut self, depth: usize) -> Result<Vec<(u32, Texture)>> {
        if depth > MAX_DEPTH {
            return Err(self.error("Texture nesting too deep"));
        }
        let list = self.chunk(14)?;
        let count = self.count(12)?;
        let mut textures = vec![];
        for _ in 0..count {
            let c = self.chunk(15)?;
            let _name = self.string()?;
            let _class = self.string()?;
            let file = self.string()?;
            self.f32()?; // amount
            let kind = self.u32()?;
            self.u32()?; // mapping type
            let offset = Vec2::new(self.f32()?, self.f32()?);
            let scale = Vec2::new(self.f32()?, self.f32()?);
            let rotation = self.f32()?;
            for _ in 0..4 {
                self.f32()?;
            } // blur, blur offset, noise, noise size
            self.u32()?;
            self.f32()?; // noise level, phase
            self.u32()?;
            self.u32()?;
            self.u32()?; // invert, reserved, filter
            let channel = self.u32()?;
            let sub = self.count(0)?;
            if sub > 0 {
                self.textures(depth + 1)?;
            }
            textures.push((
                kind,
                Texture {
                    file,
                    offset,
                    scale,
                    rotation,
                    channel,
                },
            ));
            self.finish(&c);
        }
        self.finish(&list);
        Ok(textures)
    }
    fn mesh(&mut self, name: String, parent: Option<String>, transform: Mat4) -> Result<Object> {
        let c = self.chunk(16)?;
        self.context = format!("mesh {name}: header");
        self.u32()?; // time
        let vertex_count = self.count(12)?;
        let face_count = self.count(16)?;
        let uv_count = self.count(12)?;
        let color_count = self.count(12)?;
        let material_id = self.u32()? as i32;
        let mut vertices = Vec::with_capacity(vertex_count);
        self.context = format!("mesh {name}: vertices");
        for _ in 0..vertex_count {
            vertices.push(reflect(self.vector()?));
        }
        let mut indices = Vec::with_capacity(face_count);
        for _ in 0..face_count {
            indices.push((self.indices(vertex_count)?, self.u32()? as usize));
        }
        let mut uv_vertices = vec![];
        self.context = format!("mesh {name}: UV coordinates");
        for _ in 0..uv_count {
            uv_vertices.push(self.uv()?);
        }
        let mut uv_faces = vec![];
        if uv_count > 0 {
            for _ in 0..face_count {
                uv_faces.push(self.indices(uv_count)?);
            }
        }
        let additional = self.u32()?;
        let mut uv_channels = BTreeMap::new();
        if additional > 0 {
            let count = self.count(32)?;
            for _ in 0..count {
                let id = self.u32()?;
                let vertices_count = self.count(12)?;
                self.u32()?;
                let faces_count = self.count(12)?;
                self.u32()?;
                self.u32()?;
                if self.u32()? as usize != vertices_count || self.u32()? as usize != faces_count {
                    return Err(self.error("Inconsistent additional UV channel counts"));
                }
                let mut vertices = vec![];
                for _ in 0..vertices_count {
                    vertices.push(self.uv()?);
                }
                let mut faces = vec![];
                if vertices_count > 0 {
                    for _ in 0..faces_count {
                        faces.push(self.indices(vertices_count)?);
                    }
                }
                uv_channels.insert(id, UvChannel { vertices, faces });
            }
        }
        self.context = format!("mesh {name}: vertex colors");
        for _ in 0..color_count {
            self.vector()?;
        }
        if color_count > 0 {
            for _ in 0..face_count {
                self.indices(color_count)?;
            }
        }
        let mut faces = vec![];
        self.context = format!("mesh {name}: normals");
        for (i, (vertices, material)) in indices.into_iter().enumerate() {
            self.vector()?; // face normal; keep the three corner normals
            let normals = [
                reflect(self.vector()?).normalize_or_zero(),
                reflect(self.vector()?).normalize_or_zero(),
                reflect(self.vector()?).normalize_or_zero(),
            ];
            let uv = uv_faces
                .get(i)
                .map_or([[0.0; 2]; 3], |f| f.map(|j| uv_vertices[j].to_array()));
            faces.push(Face {
                vertices,
                material,
                uv,
                normals,
            });
        }
        self.u32()?; // backface culling
        if uv_count > 0 {
            uv_channels.insert(
                1,
                UvChannel {
                    vertices: uv_vertices,
                    faces: uv_faces,
                },
            );
        }
        self.finish(&c);
        Ok(Object {
            name,
            visible: true,
            parent,
            transform,
            vertices,
            faces,
            material: material_id.max(0) as usize,
            uv_channels,
        })
    }
    fn bodies(&mut self, depth: usize) -> Result<Vec<Body>> {
        if depth > MAX_DEPTH {
            return Err(self.error("Rigid-body nesting too deep"));
        }
        let list = self.chunk(33)?;
        let count = self.count(12)?;
        let mut bodies = vec![];
        for _ in 0..count {
            let c = self.chunk(32)?;
            let name = self.string()?;
            let mass = self.f32()?;
            let restitution = self.f32()?;
            let friction = self.f32()?;
            self.f32()?;
            self.u32()?; // optimization, unyielding
            let geometry = geometry(self.u32()?);
            let proxy = self.string()?;
            let use_display_proxy = self.flag()?;
            let disabled = self.flag()?;
            let initially_sleeping = self.flag()?;
            let display_proxy = self.string()?;
            let transform = self.tm()?;
            self.u32()?; // geometry type
            let child_count = self.count(0)?;
            let children = if child_count > 0 {
                self.bodies(depth + 1)?
            } else {
                vec![]
            };
            if children.len() != child_count {
                return Err(self.error("Rigid-body child count mismatch"));
            }
            bodies.push(Body {
                name,
                transform,
                mass,
                restitution,
                friction,
                geometry,
                proxy,
                display_proxy,
                use_display_proxy,
                disabled,
                initially_sleeping,
                children,
            });
            self.finish(&c);
        }
        self.finish(&list);
        Ok(bodies)
    }
    fn object(&mut self, model: &mut Model) -> Result<()> {
        let id = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("Missing object record"))?;
        let id = u32::from_le_bytes(id.try_into().unwrap());
        let c = self.chunk(id)?;
        match id {
            2 => {
                let name = self.string()?;
                let parent = self.string()?;
                self.flag()?;
                let transform = self.tm()?;
                model.objects.push(self.mesh(
                    name,
                    match parent.as_str() {
                        "" | "(null)" => None,
                        _ => Some(parent),
                    },
                    transform,
                )?);
            }
            21 => {
                let name = self.string()?;
                let transform = self.tm()?;
                let properties = properties(&self.string()?);
                model.attachments.push(Attachment {
                    name,
                    transform,
                    properties,
                });
            }
            31 => {
                self.string()?;
                let pairs = self.count(0)?;
                self.u32()?;
                model.bodies.extend(self.bodies(0)?);
                if pairs > 0 {
                    let pair_chunk = self.chunk(51)?;
                    let count = self.count(8)?;
                    if count != pairs {
                        return Err(self.error("Disabled pair count mismatch"));
                    }
                    for _ in 0..count {
                        self.string()?;
                        self.string()?;
                    }
                    self.extension(&pair_chunk);
                    self.finish(&pair_chunk);
                }
            }
            // Read known schemas to establish a trustworthy boundary and retain exact source bytes.
            4 => {
                self.string()?;
                self.tm()?;
                if self.u32()? != 0 {
                    self.tm()?;
                }
                self.u32()?;
                for _ in 0..6 {
                    self.f32()?;
                }
                self.extension(&c);
            }
            5 => {
                self.string()?;
                self.tm()?;
                let target = self.string()?;
                if !target.is_empty() {
                    self.tm()?;
                }
                for _ in 0..3 {
                    self.u32()?;
                }
                self.take(4)?;
                self.f32()?;
                self.f32()?;
                self.take(8)?;
                for _ in 0..3 {
                    self.f32()?;
                }
                self.take(4)?;
                self.extension(&c);
            }
            30 => {
                self.string()?;
                self.vector()?;
                self.f32()?;
                self.f32()?;
                self.u32()?;
                self.flag()?;
                self.f32()?;
                self.f32()?;
                self.flag()?;
                self.f32()?;
                self.f32()?;
                self.flag()?;
                self.f32()?;
                self.u32()?;
                self.extension(&c);
            }
            42 => {
                self.string()?;
                self.f32()?;
                self.string()?;
                let count = self.count(0)?;
                if count > 0 {
                    let list = self.chunk(44)?;
                    if self.count(12)? != count {
                        return Err(self.error("Constraint count mismatch"));
                    }
                    for _ in 0..count {
                        let id = self.u32()?;
                        self.pos -= 4;
                        let constraint = self.chunk(id)?;
                        if ![46, 47, 53].contains(&id) {
                            return Err(self.error(format!("Unknown constraint type {id}")));
                        }
                        let name = self.string()?;
                        self.tm()?;
                        let bodies = [self.string()?, self.string()?];
                        let pivot = reflect(self.vector()?);
                        // Angular axes are pseudovectors under the left/right handed reflection.
                        let axis = -reflect(self.vector()?).normalize_or_zero();
                        let kind = match id {
                            46 => {
                                let mut parameters = [0.0; 8];
                                for value in &mut parameters {
                                    *value = self.f32()?;
                                }
                                JointKind::Ragdoll { parameters }
                            }
                            47 => {
                                let limited = self.flag()?;
                                let friction = self.f32()?;
                                let limits = [self.f32()?, self.f32()?];
                                JointKind::Hinge {
                                    limited,
                                    friction,
                                    limits,
                                }
                            }
                            _ => JointKind::Ball,
                        };
                        model.joints.push(ModelJoint {
                            name,
                            bodies,
                            pivot,
                            axis,
                            kind,
                        });
                        self.finish(&constraint);
                    }
                    self.finish(&list);
                }
                self.extension(&c);
            }
            49 => {
                self.string()?;
                self.tm()?;
                self.string()?;
                self.string()?;
                self.f32()?;
                self.f32()?;
                self.flag()?;
                for _ in 0..4 {
                    self.f32()?;
                }
                self.extension(&c);
            }
            _ => {
                return Err(self.error(format!(
                    "Unsupported object record {id}; length field cannot be trusted"
                )));
            }
        }
        self.finish(&c);
        Ok(())
    }
}

pub fn decode(bytes: &[u8]) -> Result<ImportedModel> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("GMI exceeds the 64 MiB input limit".into());
    }
    let mut r = Reader {
        bytes,
        pos: 0,
        context: "header".into(),
        report: ImportReport {
            format: "GMI v3",
            ..Default::default()
        },
    };
    if r.take(3)? != b"GMI" || r.u32()? != 3 || r.u32()? != 1 {
        return Err(r.error("Unsupported GMI signature, version or model type"));
    }
    r.u32()?;
    r.u32()?; // reserved header fields
    let c = r.chunk(1)?;
    r.string()?;
    for _ in 0..6 {
        r.u32()?;
    } // frame range, speed, ticks, background and ambient colors
    r.finish(&c);
    let materials = r.material_list(0)?;
    let mut model = Model {
        materials,
        ..Default::default()
    };
    let c = r.chunk(18)?;
    let count = r.count(12)?;
    for _ in 0..count {
        r.object(&mut model)?;
    }
    r.finish(&c);
    if r.pos != bytes.len() {
        return Err(r.error(format!("{} unconsumed bytes", bytes.len() - r.pos)));
    }
    Ok(ImportedModel {
        model,
        report: r.report,
    })
}
