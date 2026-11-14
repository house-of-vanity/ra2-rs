//! Mixed line/binary RA2 robot saves. No regex scanning through embedded images.
pub use crate::bot_write::encode;
use arena_core::{robot::*, *};
use std::collections::BTreeMap;

const LIMIT: usize = 1_000_000;
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    minor: u32,
}
impl Reader<'_> {
    fn err(&self, message: impl std::fmt::Display) -> String {
        format!("BOT at 0x{:x}: {message}", self.pos)
    }
    fn line(&mut self) -> Result<String, String> {
        let remaining = &self.bytes[self.pos..];
        let length = remaining
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| self.err("Missing line terminator"))?;
        if length > 4096 {
            return Err(self.err("Oversized text line"));
        }
        let line = String::from_utf8_lossy(&remaining[..length])
            .trim()
            .to_string();
        self.pos += length + 1;
        Ok(line)
    }
    fn values<T: std::str::FromStr>(&mut self) -> Result<Vec<T>, String> {
        self.line()?
            .split_whitespace()
            .map(|v| {
                v.parse()
                    .map_err(|_| self.err(format!("Invalid value {v}")))
            })
            .collect()
    }
    fn int(&mut self) -> Result<i32, String> {
        self.line()?
            .parse()
            .map_err(|_| self.err("Expected integer"))
    }
    fn count(&mut self) -> Result<usize, String> {
        let n = self.index()?;
        if n > self.bytes.len() - self.pos {
            return Err(self.err("Count exceeds remaining input"));
        }
        Ok(n)
    }
    fn index(&mut self) -> Result<usize, String> {
        let n = self.int()?;
        if n < 0 || n as usize > LIMIT {
            return Err(self.err("Invalid count"));
        }
        Ok(n as usize)
    }
    fn boolean(&mut self) -> Result<bool, String> {
        match self.line()?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(self.err("Expected true/false")),
        }
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let v = self.values::<f32>()?;
        if v.len() != N || v.iter().any(|v| !v.is_finite()) {
            return Err(self.err(format!("Expected {N} finite numbers")));
        }
        Ok(v.try_into().unwrap())
    }
    fn image(&mut self, size: usize) -> Result<Vec<u8>, String> {
        let length = 18 + 4 * size * size;
        let bytes = self
            .bytes
            .get(self.pos..self.pos + length)
            .ok_or_else(|| self.err("Truncated TGA image"))?
            .to_vec();
        if bytes[0] != 0
            || bytes[2] != 2
            || bytes[16] != 32
            || u16::from_le_bytes([bytes[12], bytes[13]]) as usize != size
            || u16::from_le_bytes([bytes[14], bytes[15]]) as usize != size
        {
            return Err(self.err("Unsupported embedded TGA layout"));
        }
        self.pos += length;
        Ok(bytes)
    }
    fn mesh(&mut self) -> Result<Model, String> {
        let header = self.values::<usize>()?;
        if header.len() != 4
            || header[0] != 274
            || header[1] > LIMIT
            || header[2] > LIMIT
            || header[3] > LIMIT
        {
            return Err(self.err("Invalid mesh header"));
        }
        if self.count()? != 1 {
            return Err(self.err("Only one embedded mesh is supported"));
        }
        let counts = self.values::<usize>()?;
        if counts.len() != 4
            || counts[0] != header[2]
            || counts[1] != header[3]
            || counts[2] != header[1]
            || counts[0] != counts[1] * 3
            || counts[3] != 0
            || self.line()? != "RAW"
        {
            return Err(self.err("Inconsistent mesh counts/layout"));
        }
        let n = counts[2];
        if n.saturating_mul(12) > self.bytes.len() - self.pos {
            return Err(self.err("Embedded mesh exceeds remaining data"));
        }
        let mut normals = vec![];
        let mut uv = vec![];
        let mut vertices = vec![];
        for _ in 0..n {
            normals.push(super::reflect(Vec3::from_array(self.floats()?)).normalize_or_zero());
        }
        for _ in 0..n {
            let [u, v] = self.floats()?;
            // BOT chassis UVs already use top-left texture coordinates, unlike GMI.
            uv.push([u, v]);
        }
        for _ in 0..n {
            vertices.push(super::reflect(Vec3::from_array(self.floats()?)));
        }
        let mut faces = vec![];
        for _ in 0..counts[1] {
            let mut ids = [self.index()?, self.index()?, self.index()?];
            if ids.iter().any(|i| *i >= n) {
                return Err(self.err("Mesh index outside vertex array"));
            }
            let [a, b, c] = ids.map(|i| vertices[i]);
            if (b - a).cross(c - a).dot(normals[ids[0]]) < 0.0 {
                ids.swap(1, 2);
            }
            faces.push(Face {
                vertices: ids,
                material: 0,
                uv: ids.map(|i| uv[i]),
                normals: ids.map(|i| normals[i]),
            });
        }
        let material = Material {
            name: "Chassis paint".into(),
            color: [0.65; 3],
            alpha: 1.0,
            two_sided: false,
            texture: None,
            opacity_texture: None,
            submaterials: BTreeMap::new(),
        };
        let model = Model {
            materials: BTreeMap::from([(0, material)]),
            objects: vec![Object {
                name: "chassis".into(),
                visible: true,
                parent: None,
                transform: Mat4::IDENTITY,
                vertices,
                faces,
                material: 0,
                uv_channels: BTreeMap::new(),
            }],
            ..Default::default()
        };
        model.validate()?;
        Ok(model)
    }
    fn chassis(&mut self) -> Result<Chassis, String> {
        let mut bottom = vec![];
        for _ in 0..self.count()? {
            bottom.push(Vec2::from_array(self.floats()?));
        }
        let mut top = vec![];
        for _ in 0..self.count()? {
            top.push(Vec2::from_array(self.floats()?));
        }
        let [height, face_count] = self.floats()?;
        let mut paint_layout = ChassisPaintLayout::default();
        let mut model = None;
        let mut texture = None;
        if self.boolean()? {
            model = Some(self.mesh()?);
            if self.boolean()? {
                texture = Some(self.image(256)?);
            }
            if self.minor >= 8 && self.boolean()? {
                return Err(self.err("Unsupported chassis extension flag"));
            }
            for _ in 0..self.count()? {
                let mut group = vec![];
                for _ in 0..self.count()? {
                    group.push(Vec2::from_array(self.floats::<2>()?));
                }
                paint_layout.group_uvs.push(group);
            }
            for _ in 0..self.count()? {
                let row: [i32; 5] = self
                    .values::<i32>()?
                    .try_into()
                    .map_err(|_| self.err("Invalid texture group bounds"))?;
                paint_layout.rectangles.push(row);
            }
            for _ in 0..self.count()? {
                let row: [i32; 4] = self
                    .values::<i32>()?
                    .try_into()
                    .map_err(|_| self.err("Invalid texture face group"))?;
                paint_layout.face_groups.push(row);
            }
            for _ in 0..self.count()? {
                let row: [i32; 5] = self
                    .values::<i32>()?
                    .try_into()
                    .map_err(|_| self.err("Invalid top corner"))?;
                paint_layout.top_corners.push(row);
            }
        }
        let armour = self.values::<f32>()?;
        if armour.len() != 5 || armour.iter().any(|v| !v.is_finite()) {
            return Err(self.err("Invalid armour data"));
        }
        let material_parameters = if model.is_some() {
            self.floats::<5>()?
        } else {
            [0.2, 0.2, 0.2, 0.85, 20.0]
        };
        let mut buckling_points = vec![];
        if self.minor >= 11 {
            for _ in 0..self.count()? {
                let position = super::reflect(Vec3::from_array(self.floats()?));
                let mut indices = vec![];
                for _ in 0..self.count()? {
                    indices.push(self.index()?);
                }
                buckling_points.push((position, indices));
            }
        }
        if let Some(model) = &mut model
            && texture.is_some()
        {
            model.materials.get_mut(&0).unwrap().texture = Some(Texture {
                file: "@chassis.tga".into(),
                scale: Vec2::ONE,
                offset: Vec2::ZERO,
                rotation: 0.0,
                channel: 1,
            });
        }
        Ok(Chassis {
            bottom,
            top,
            height,
            model,
            texture,
            armour: armour[0] as u32,
            surface_density: armour[1],
            hardness: armour[3],
            buckling_points,
            face_count: face_count as usize,
            paint_layout,
            material_parameters,
            armour_flags: [armour[2], armour[4]],
        })
    }
}

pub fn decode(bytes: &[u8]) -> Result<RobotDefinition, String> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("BOT exceeds 64 MiB".into());
    }
    let mut r = Reader {
        bytes,
        pos: 0,
        minor: 0,
    };
    let version = r.line()?;
    r.minor = match version.as_str() {
        "1.07" => 7,
        "1.08" => 8,
        "1.11" => 11,
        "1.12" => 12,
        _ => return Err(r.err(format!("Unsupported robot version {version}"))),
    };
    let name = r
        .line()?
        .strip_prefix("Name:")
        .ok_or("Missing robot Name:")?
        .trim()
        .to_string();
    let weight_class = r
        .line()?
        .strip_prefix("Class:")
        .ok_or("Missing robot Class:")?
        .trim()
        .parse()
        .map_err(|_| "Invalid weight class")?;
    let default_paint = if r.minor >= 12 { r.int()? } else { 0 };
    let snapshot = if r.boolean()? {
        Some(r.image(128)?)
    } else {
        None
    };
    let mut components = vec![];
    let mut chassis = None;
    for index in 0..r.count()? {
        let parent = r.int()?;
        let parent_mount = r.index()? as u32;
        let own_mount = r.index()? as u32;
        let translation = Vec3::from_array(r.floats()?);
        let rotation = r.floats::<4>()?;
        let [angle, height] = r.floats()?;
        let class = r.line()?;
        let definition = r.line()?;
        // Contrary to early format notes, shipped saves use XYZW quaternions and Y-up positions.
        let quaternion = arena_core::Quat::from_array(rotation);
        if quaternion.length_squared() < 1e-12 {
            return Err(r.err("Zero component quaternion"));
        }
        let q = quaternion.normalize();
        let mounting = MountPlacement {
            position: super::reflect(translation),
            rotation: Quat::from_xyzw(-q.x, -q.y, q.z, q.w),
            twist: std::f32::consts::PI - angle,
            offset: height,
        };
        if !mounting.matrix().is_finite() {
            return Err(r.err("Invalid component rotation"));
        }
        let mut settings = BTreeMap::new();
        match class.as_str() {
            "Chassis" => {
                if index != 0 {
                    return Err(r.err("Chassis must be first"));
                }
                chassis = Some(r.chassis()?);
            }
            "BurstMotor" => {
                let [start, end] = r.floats()?;
                settings.insert("start_angle".into(), start.to_string());
                settings.insert("stop_angle".into(), end.to_string());
            }
            "SmartZone" => {
                settings.insert("zone".into(), r.line()?);
            }
            _ => (),
        }
        if parent < -1 {
            return Err(r.err("Invalid parent index"));
        }
        components.push(RobotComponent {
            parent: if parent < 0 {
                None
            } else {
                Some(parent as usize)
            },
            parent_mount,
            own_mount,
            mounting,
            class,
            definition,
            settings,
        });
    }
    if components.is_empty() {
        return Err(r.err("Empty robot"));
    }
    if components[0].parent.is_some() {
        return Err(r.err("Chassis must be the assembly root"));
    }
    for (i, c) in components.iter().enumerate() {
        if c.parent.is_some_and(|p| p >= components.len() || p == i) {
            return Err(r.err("Invalid robot parent"));
        }
    }
    let damaged_model = if r.minor >= 11 && r.boolean()? {
        Some(r.mesh()?)
    } else {
        None
    };
    let heading = if r.minor >= 11 {
        r.floats::<1>()?[0]
    } else {
        0.0
    };
    let mut controllers = vec![];
    for _ in 0..r.count()? {
        let name = r.line()?;
        let address = r.values::<i32>()?;
        if address.len() != 3 {
            return Err(r.err("Invalid controller address"));
        }
        let mut keys = vec![];
        for _ in 0..r.count()? {
            let key = r.values::<i32>()?;
            if !(3..=4).contains(&key.len()) {
                return Err(r.err("Invalid controller key"));
            }
            keys.push(key);
        }
        controllers.push(Controller {
            name,
            position: [address[0], address[1]],
            kind: address[2] as u32,
            keys,
        });
    }
    let mut bindings = vec![];
    for _ in 0..r.count()? {
        let component = r.index()?;
        let action = r.line()?;
        let address = r.values::<i32>()?;
        if address.len() != 3 || component >= components.len() || address[2] < 0 {
            return Err(r.err("Invalid control binding"));
        }
        bindings.push(ControlBinding {
            component,
            action,
            controller: [address[0], address[1]],
            key: address[2] as usize,
        });
    }
    if r.bytes[r.pos..].iter().any(|b| !b.is_ascii_whitespace()) {
        return Err(r.err("Unconsumed robot data"));
    }
    Ok(RobotDefinition {
        name,
        source_version: version,
        weight_class,
        components,
        chassis: chassis.ok_or("Robot has no chassis")?,
        snapshot,
        damaged_model,
        heading,
        controllers,
        bindings,
        default_paint,
    })
}
