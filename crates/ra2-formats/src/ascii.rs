//! ASCII GMA v3 decoder. Local geometry and absolute model transforms remain separate.
use std::collections::BTreeMap;

use crate::{geometry, properties, right_handed};
use arena_core::*;

#[derive(Debug)]
struct Node {
    key: String,
    value: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, key: &str) -> Option<&Self> {
        self.children.iter().find(|n| n.key == key)
    }
    fn value(&self, key: &str) -> &str {
        self.child(key).map_or("", |n| n.value.as_str())
    }
    fn number(&self, key: &str, fallback: f32) -> f32 {
        self.value(key).parse().unwrap_or(fallback)
    }
    fn descendants<'a>(&'a self, key: &str, output: &mut Vec<&'a Self>) {
        for child in &self.children {
            if child.key == key {
                output.push(child);
            }
            child.descendants(key, output);
        }
    }
}

fn tree(text: &str) -> Result<Node, String> {
    let mut stack = vec![Node {
        key: String::new(),
        value: String::new(),
        children: vec![],
    }];
    for (line, raw) in text.lines().enumerate() {
        let raw = raw.trim();
        match raw {
            "" => (),
            "{" => {
                let node = stack
                    .last_mut()
                    .unwrap()
                    .children
                    .pop()
                    .ok_or_else(|| format!("Block without a name at line {}", line + 1))?;
                if stack.len() > 64 {
                    return Err("GMF nesting is too deep".into());
                }
                stack.push(node);
            }
            "}" => {
                if stack.len() == 1 {
                    return Err(format!("Unexpected closing brace at line {}", line + 1));
                }
                let node = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(node);
            }
            _ => {
                let (key, value) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
                stack.last_mut().unwrap().children.push(Node {
                    key: key.into(),
                    value: value.trim().into(),
                    children: vec![],
                });
            }
        }
    }
    if stack.len() != 1 {
        return Err("Unclosed GMF block".into());
    }
    Ok(stack.pop().unwrap())
}

fn numbers<T: std::str::FromStr>(value: &str) -> Result<Vec<T>, String> {
    value
        .split_whitespace()
        .map(|s| s.parse().map_err(|_| format!("Invalid GMF number: {s}")))
        .collect()
}

fn vector(value: &str) -> Result<Vec3, String> {
    let v = numbers::<f32>(value)?;
    if v.len() < 3 || !v.iter().all(|v| v.is_finite()) {
        return Err(format!("Invalid GMF vector: {value}"));
    }
    Ok(Vec3::new(v[0], v[1], v[2]))
}

fn reflected(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, -v.z)
}

fn matrix(node: &Node) -> Result<Mat4, String> {
    let Some(tm) = node.child("*NODE_TM") else {
        return Ok(Mat4::IDENTITY);
    };
    Ok(Mat4::from_cols(
        vector(tm.value("*TM_ROW0"))?.extend(0.0),
        vector(tm.value("*TM_ROW1"))?.extend(0.0),
        vector(tm.value("*TM_ROW2"))?.extend(0.0),
        vector(tm.value("*TM_ROW3"))?.extend(1.0),
    ))
}

fn materials(node: Option<&Node>) -> BTreeMap<usize, Material> {
    let Some(list) = node else {
        return BTreeMap::new();
    };
    list.children
        .iter()
        .filter(|n| n.key == "*MATERIAL")
        .map(|n| {
            // Max's color integers are COLORREF: low byte = red.
            let color =
                u32::from_str_radix(n.value("*MATERIAL_DIFFUSE").trim_start_matches("0x"), 16)
                    .unwrap_or(0xcccccc);
            let texture = n
                .child("*TEXTURE_LIST")
                .and_then(|list| {
                    list.children
                        .iter()
                        .find(|n| n.child("*MAP_DIFFUSE").is_some())
                })
                .map(|t| Texture {
                    file: t.value("*BITMAP").to_string(),
                    scale: Vec2::new(
                        t.number("*UVW_U_TILING", 1.0),
                        t.number("*UVW_V_TILING", 1.0),
                    ),
                    rotation: t.number("*UVW_ANGLE", 0.0),
                    channel: t.number("*BITMAP_MAP_CHANNEL", 1.0) as u32,
                    offset: Vec2::new(
                        t.number("*UVW_U_OFFSET", 0.0),
                        t.number("*UVW_V_OFFSET", 0.0),
                    ),
                });
            (
                n.number("*MATERIAL_REF_NO", 0.0) as usize,
                Material {
                    name: n.value("*MATERIAL_NAME").into(),
                    color: [
                        (color & 255) as f32 / 255.0,
                        ((color >> 8) & 255) as f32 / 255.0,
                        ((color >> 16) & 255) as f32 / 255.0,
                    ],
                    alpha: 1.0 - n.number("*MATERIAL_TRANSPARENCY", 0.0).clamp(0.0, 1.0),
                    texture,
                    opacity_texture: n
                        .child("*TEXTURE_LIST")
                        .and_then(|l| {
                            l.children
                                .iter()
                                .find(|t| t.child("*MAP_OPACITY").is_some())
                        })
                        .map(|t| Texture {
                            file: t.value("*BITMAP").into(),
                            scale: Vec2::ONE,
                            offset: Vec2::ZERO,
                            rotation: 0.0,
                            channel: t.number("*BITMAP_MAP_CHANNEL", 1.0) as u32,
                        }),
                    two_sided: n.number("*MATERIAL_TWOSIDED", 0.0) != 0.0,
                    submaterials: materials(n.child("*MATERIAL_LIST")),
                },
            )
        })
        .collect()
}

fn object(node: &Node) -> Result<Option<Object>, String> {
    let Some(mesh) = node.child("*MESH") else {
        return Ok(None);
    };
    let Some(vertices) = mesh.child("*MESH_VERTEX_LIST") else {
        return Ok(None);
    };
    let transform = matrix(node)?;
    let normal_transform = Mat3::from_mat4(transform).inverse().transpose();
    if !normal_transform.is_finite() {
        return Err("Singular object transform".into());
    }
    let count = mesh.number("*MESH_NUMVERTEX", 0.0);
    if !(0.0..=2_000_000.0).contains(&count) {
        return Err("Invalid vertex count".into());
    }
    let mut positions = vec![Vec3::ZERO; count as usize];
    for v in &vertices.children {
        let (id, value) = v
            .value
            .split_once(char::is_whitespace)
            .ok_or("Invalid vertex")?;
        let id: usize = id.parse().map_err(|_| "Invalid vertex index")?;
        *positions.get_mut(id).ok_or("Vertex index out of range")? =
            reflected(vector(value.trim())?);
    }
    let mut uvs = BTreeMap::new();
    if let Some(list) = mesh.child("*MESH_TVERTLIST") {
        for v in &list.children {
            let data = numbers::<f32>(&v.value)?;
            if data.len() != 4 {
                return Err("Invalid texture vertex".into());
            }
            uvs.insert(data[0] as usize, [data[1], 1.0 - data[2]]);
        }
    }
    let mut uv_faces = BTreeMap::new();
    if let Some(list) = mesh.child("*MESH_TFACELIST") {
        for f in &list.children {
            let data = numbers::<usize>(&f.value)?;
            if data.len() != 4 {
                return Err("Invalid texture face".into());
            }
            uv_faces.insert(data[0], [data[1], data[2], data[3]]);
        }
    }
    let mut normals: BTreeMap<usize, Vec<Vec3>> = BTreeMap::new();
    if let Some(list) = mesh.child("*MESH_NORMALS") {
        let mut face = 0;
        for n in &list.children {
            let (id, value) = n
                .value
                .split_once(char::is_whitespace)
                .ok_or("Invalid normal")?;
            if n.key == "*MESH_FACENORMAL" {
                face = id.parse().map_err(|_| "Invalid normal face index")?;
            } else if n.key == "*MESH_VERTEXNORMAL" {
                normals
                    .entry(face)
                    .or_default()
                    .push(reflected(vector(value.trim())?).normalize_or_zero());
            }
        }
    }
    let mut faces = vec![];
    if let Some(list) = mesh.child("*MESH_FACE_LIST") {
        for f in &list.children {
            let data: Vec<_> = f.value.split_whitespace().collect();
            if data.len() != 9
                || data[1] != "A:"
                || data[3] != "B:"
                || data[5] != "C:"
                || data[7] != "*MESH_MTLID"
            {
                return Err(format!("Unsupported GMF face: {}", f.value));
            }
            let index = |i: usize| {
                data[i]
                    .parse::<usize>()
                    .map_err(|_| "Invalid face index".to_string())
            };
            let id = index(0)?;
            let vertices = [index(2)?, index(4)?, index(6)?];
            if vertices.iter().any(|&v| v >= positions.len()) {
                return Err("Face vertex out of range".into());
            }
            let [a, b, c] = vertices.map(|i| positions[i]);
            let fallback = (b - a).cross(c - a).normalize_or_zero();
            let ns = normals.get(&id);
            faces.push(Face {
                vertices,
                material: index(8)?,
                uv: if let Some(face) = uv_faces.get(&id) {
                    let mut uv = [[0.0; 2]; 3];
                    for i in 0..3 {
                        uv[i] = *uvs.get(&face[i]).ok_or("Texture vertex out of range")?;
                    }
                    uv
                } else {
                    [[0.0; 2]; 3]
                },
                normals: std::array::from_fn(|i| {
                    ns.and_then(|n| n.get(i)).copied().unwrap_or(fallback)
                }),
            });
        }
    }
    let mut uv_channels = BTreeMap::new();
    if !uvs.is_empty() {
        uv_channels.insert(
            1,
            UvChannel {
                vertices: uvs.values().map(|uv| Vec2::from_array(*uv)).collect(),
                faces: uv_faces.values().copied().collect(),
            },
        );
    }
    for channel in mesh
        .children
        .iter()
        .filter(|n| n.key == "*MESH_MAPPINGCHANNEL")
    {
        let id = channel.value.parse().map_err(|_| "Invalid UV channel ID")?;
        let mut vertices = vec![];
        if let Some(list) = channel.child("*MESH_TVERTLIST") {
            for v in &list.children {
                let data = numbers::<f32>(&v.value)?;
                if data.len() != 4 || data[0] as usize != vertices.len() {
                    return Err("Invalid UV channel vertex".into());
                }
                vertices.push(Vec2::new(data[1], 1.0 - data[2]));
            }
        }
        let mut faces = vec![];
        if let Some(list) = channel.child("*MESH_TFACELIST") {
            for f in &list.children {
                let data = numbers::<usize>(&f.value)?;
                if data.len() != 4 || data[1..].iter().any(|i| *i >= vertices.len()) {
                    return Err("Invalid UV channel face".into());
                }
                faces.push([data[1], data[2], data[3]]);
            }
        }
        uv_channels.insert(id, UvChannel { vertices, faces });
    }
    Ok(Some(Object {
        name: node.value("*NODE_NAME").into(),
        visible: true,
        parent: match node.value("*NODE_PARENT") {
            "" | "(null)" => None,
            name => Some(name.into()),
        },
        transform: right_handed(transform),
        uv_channels,
        vertices: positions,
        faces,
        material: mesh.number("*MATERIAL_REF", 0.0).max(0.0) as usize,
    }))
}

fn read_body(n: &Node) -> Result<Body, String> {
    let children = match n.child("*GMID_HAVOK_RIGIDBODY_LIST") {
        Some(list) => list
            .children
            .iter()
            .filter(|n| n.key == "*GMID_HAVOK_RIGIDBODY")
            .map(read_body)
            .collect::<Result<_, _>>()?,
        None => vec![],
    };
    Ok(Body {
        name: n.value("*NODE_NAME").into(),
        proxy: n.value("*GEOMETRY_PROXY_NAME").into(),
        display_proxy: n.value("*DISPLAY_PROXY_NAME").into(),
        disabled: n.value("*DISABLE_COLLISIONS") == "1",
        transform: right_handed(matrix(n)?),
        mass: n.number("*MASS", 0.0),
        geometry: geometry(n.number("*SIMULATION_GEOMETRY", 0.0) as u32),
        use_display_proxy: n.value("*USE_DISPLAY_PROXY") == "1",
        initially_sleeping: n.value("*INACTIVE") == "1",
        children,
        friction: n.number("*FRICTION", 0.7),
        restitution: n.number("*ELASTICITY", 0.0),
    })
}

pub fn parse(text: &str) -> Result<Model, String> {
    let root = tree(text)?;
    if root.child("GMA").is_none() || root.value("*GABRIEL_ASCIIEXPORT") != "3" {
        return Err("Expected ASCII GMF: GMA / GABRIEL_ASCIIEXPORT 3".into());
    }
    let mut nodes = vec![];
    root.descendants("*GEOMOBJECT", &mut nodes);
    let objects = nodes
        .into_iter()
        .map(object)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    nodes = vec![];
    root.descendants("*GMID_HAVOK_RBCOLLECTION", &mut nodes);
    let mut bodies = vec![];
    for collection in nodes {
        if let Some(list) = collection.child("*GMID_HAVOK_RIGIDBODY_LIST") {
            for body in list
                .children
                .iter()
                .filter(|n| n.key == "*GMID_HAVOK_RIGIDBODY")
            {
                bodies.push(read_body(body)?);
            }
        }
    }
    let mut attachments = vec![];
    nodes = vec![];
    root.descendants("*GMID_ATTACHMENTPT", &mut nodes);
    for n in nodes {
        let data = n
            .children
            .iter()
            .filter(|n| n.key != "*NODE_TM" && n.key != "*NODE_NAME")
            .map(|n| {
                format!(
                    "{} {}",
                    if n.key == "USER" { "" } else { &n.key },
                    n.value.trim_start_matches("DATA").trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        attachments.push(Attachment {
            name: n.value("*NODE_NAME").into(),
            transform: right_handed(matrix(n)?),
            properties: properties(&data),
        });
    }
    Ok(Model {
        materials: materials(root.child("*MATERIAL_LIST")),
        objects,
        bodies,
        attachments,
        joints: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn rejects_binary_and_unbalanced_files() {
        assert!(parse("GMI\u{3}").is_err());
        assert!(parse("GMA\n*GABRIEL_ASCIIEXPORT 3\n*OBJECT_LIST\n{\n").is_err());
        assert!(parse("}\n").is_err());
    }

    #[test]
    #[ignore = "requires a local RA2 installation"]
    fn combat_geometry_materials_uvs_and_handedness() {
        let root = std::env::var_os("RA2_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../RobotArena2"));
        let path = root.join("Arenas/box/boxarena_txt.gmf");
        let model = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(model.materials.len(), 11);
        assert_eq!(model.bodies.len(), 7);
        assert_eq!(model.materials[&10].submaterials.len(), 3);
        let floor = model.object("floor").unwrap();
        assert_eq!((floor.vertices.len(), floor.faces.len()), (18, 28));
        assert!(floor.positions().all(|v| (v.y + 2.26675).abs() < 0.001));
        for f in &floor.faces {
            let [a, b, c] = f.vertices.map(|i| floor.vertices[i]);
            assert!((b - a).cross(c - a).normalize().dot(f.normals[0]) > 0.99);
        }
        assert!(floor.faces.iter().flat_map(|f| f.uv).any(|uv| uv[0] > 1.0));
        assert!(model.object("arena_collision").is_some());
    }
}
