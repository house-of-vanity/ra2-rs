//! Native BOT serialization; no renderer, filesystem or executable dependencies.
use arena_core::{Model, Vec3, robot::RobotDefinition};

struct Writer(Vec<u8>);
impl Writer {
    fn line(&mut self, value: impl std::fmt::Display) -> Result<(), String> {
        let text = value.to_string();
        if text.contains(['\n', '\r', '\0']) {
            return Err("BOT text contains a line break or NUL".into());
        }
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(b'\n');
        Ok(())
    }
    fn row<T: std::fmt::Display>(
        &mut self,
        values: impl IntoIterator<Item = T>,
    ) -> Result<(), String> {
        self.line(
            values
                .into_iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
    fn image(&mut self, bytes: &Option<Vec<u8>>, size: usize) -> Result<(), String> {
        self.line(bytes.is_some())?;
        if let Some(bytes) = bytes {
            if bytes.len() != 18 + size * size * 4
                || bytes[2] != 2
                || bytes[16] != 32
                || u16::from_le_bytes([bytes[12], bytes[13]]) as usize != size
                || u16::from_le_bytes([bytes[14], bytes[15]]) as usize != size
            {
                return Err("Invalid embedded BOT TGA".into());
            }
            self.0.extend_from_slice(bytes);
        }
        Ok(())
    }
    fn mesh(&mut self, model: &Model) -> Result<(), String> {
        model.validate()?;
        if model.objects.len() != 1 {
            return Err("BOT chassis requires one mesh".into());
        }
        let object = &model.objects[0];
        let n = object.vertices.len();
        let faces = object.faces.len();
        let mut normals = vec![Vec3::Y; n];
        let mut uvs = vec![[0.0; 2]; n];
        let mut seen = vec![false; n];
        for face in &object.faces {
            for corner in 0..3 {
                let id = face.vertices[corner];
                let normal = face.normals[corner];
                let uv = face.uv[corner];
                if seen[id] && ((normals[id] - normal).length() > 0.0001 || uvs[id] != uv) {
                    return Err("BOT mesh needs separate vertices at UV/normal seams".into());
                }
                seen[id] = true;
                normals[id] = normal;
                uvs[id] = uv;
            }
        }
        self.row([274, n, faces * 3, faces])?;
        self.line(1)?;
        self.row([faces * 3, faces, n, 0])?;
        self.line("RAW")?;
        for normal in normals {
            self.row(super::reflect(normal).to_array())?;
        }
        for uv in uvs {
            self.row(uv)?;
        }
        for position in object.positions() {
            self.row(super::reflect(position).to_array())?;
        }
        for face in &object.faces {
            for index in [face.vertices[0], face.vertices[2], face.vertices[1]] {
                self.line(index)?;
            }
        }
        Ok(())
    }
}

pub fn encode(robot: &RobotDefinition) -> Result<Vec<u8>, String> {
    let minor = match robot.source_version.as_str() {
        "1.07" => 7,
        "1.08" => 8,
        "1.11" => 11,
        "1.12" => 12,
        _ => return Err("Unsupported BOT version for writing".into()),
    };
    let mut w = Writer(vec![]);
    w.line(&robot.source_version)?;
    w.line(format!("Name: {}", robot.name))?;
    w.line(format!("Class: {}", robot.weight_class))?;
    if minor >= 12 {
        w.line(robot.default_paint)?;
    }
    w.image(&robot.snapshot, 128)?;
    w.line(robot.components.len())?;
    for (index, component) in robot.components.iter().enumerate() {
        w.line(component.parent.map(|p| p as i32).unwrap_or(-1))?;
        w.line(component.parent_mount)?;
        w.line(component.own_mount)?;
        let m = &component.mounting;
        w.row(super::reflect(m.position).to_array())?;
        let q = m.rotation;
        w.row([-q.x, -q.y, q.z, q.w])?;
        w.row([std::f32::consts::PI - m.twist, m.offset])?;
        w.line(&component.class)?;
        w.line(component.definition.replace('/', "\\"))?;
        match component.class.as_str() {
            "Chassis" if index == 0 => {
                let c = &robot.chassis;
                w.line(c.bottom.len())?;
                for p in &c.bottom {
                    w.row([p.x.round() as i32, p.y.round() as i32])?;
                }
                w.line(c.top.len())?;
                for p in &c.top {
                    w.row([p.x.round() as i32, p.y.round() as i32])?;
                }
                w.line(format!("{} {}", c.height, c.face_count))?;
                w.line(c.model.is_some())?;
                if let Some(model) = &c.model {
                    w.mesh(model)?;
                    w.image(&c.texture, 256)?;
                    if minor >= 8 {
                        w.line(false)?;
                    }
                    let paint = &c.paint_layout;
                    w.line(paint.group_uvs.len())?;
                    for group in &paint.group_uvs {
                        w.line(group.len())?;
                        for uv in group {
                            w.row(uv.to_array())?;
                        }
                    }
                    w.line(paint.rectangles.len())?;
                    for row in &paint.rectangles {
                        w.row(*row)?;
                    }
                    w.line(paint.face_groups.len())?;
                    for row in &paint.face_groups {
                        w.row(*row)?;
                    }
                    w.line(paint.top_corners.len())?;
                    for row in &paint.top_corners {
                        w.row(*row)?;
                    }
                }
                w.line(format!(
                    "{} {} {} {} {}",
                    c.armour, c.surface_density, c.armour_flags[0], c.hardness, c.armour_flags[1]
                ))?;
                if c.model.is_some() {
                    w.row(c.material_parameters)?;
                }
                if minor >= 11 {
                    w.line(c.buckling_points.len())?;
                    for (position, indices) in &c.buckling_points {
                        w.row(super::reflect(*position).to_array())?;
                        w.line(indices.len())?;
                        for index in indices {
                            w.line(index)?;
                        }
                    }
                }
            }
            "BurstMotor" => w.line(format!(
                "{} {}",
                component
                    .settings
                    .get("start_angle")
                    .ok_or("Missing burst start angle")?,
                component
                    .settings
                    .get("stop_angle")
                    .ok_or("Missing burst stop angle")?
            ))?,
            "SmartZone" => w.line(
                component
                    .settings
                    .get("zone")
                    .ok_or("Missing zone settings")?,
            )?,
            _ => (),
        }
    }
    if minor >= 11 {
        w.line(robot.damaged_model.is_some())?;
        if let Some(model) = &robot.damaged_model {
            w.mesh(model)?;
        }
        w.line(robot.heading)?;
    }
    w.line(robot.controllers.len())?;
    for controller in &robot.controllers {
        w.line(&controller.name)?;
        w.row([
            controller.position[0],
            controller.position[1],
            controller.kind as i32,
        ])?;
        w.line(controller.keys.len())?;
        for key in &controller.keys {
            w.row(key)?;
        }
    }
    w.line(robot.bindings.len())?;
    for binding in &robot.bindings {
        w.line(binding.component)?;
        w.line(&binding.action)?;
        w.row([
            binding.controller[0],
            binding.controller[1],
            binding.key as i32,
        ])?;
    }
    // Validate the complete stream before exposing it to a filesystem adapter.
    super::bot::decode(&w.0)?;
    Ok(w.0)
}
