//! Scene-only group nodes carry parent-local transforms in native exports.
//! Flatten empty grouping nodes before rendering; component binding is separate.
use arena_core::{Mat4, Model};

pub fn display_groups(source: &Model) -> Result<Model, String> {
    fn solve(
        i: usize,
        model: &Model,
        states: &mut [u8],
        frames: &mut [Mat4],
        depth: usize,
    ) -> Result<Mat4, String> {
        if states[i] == 2 {
            return Ok(frames[i]);
        }
        if states[i] == 1 || depth > 128 {
            return Err("Cyclic scene display group".into());
        }
        states[i] = 1;
        let object = &model.objects[i];
        let group = object
            .parent
            .as_ref()
            .and_then(|name| model.objects.iter().position(|o| o.name == *name))
            .filter(|&p| model.objects[p].vertices.is_empty() && model.objects[p].faces.is_empty());
        frames[i] = if let Some(parent) = group {
            solve(parent, model, states, frames, depth + 1)? * object.transform
        } else {
            object.transform
        };
        states[i] = 2;
        Ok(frames[i])
    }
    let mut result = source.clone();
    let mut states = vec![0; source.objects.len()];
    let mut frames = vec![Mat4::IDENTITY; source.objects.len()];
    for (i, object) in result.objects.iter_mut().enumerate() {
        object.transform = solve(i, source, &mut states, &mut frames, 0)?;
        if object.parent.as_ref().is_some_and(|n| {
            source
                .objects
                .iter()
                .any(|o| o.name == *n && o.vertices.is_empty() && o.faces.is_empty())
        }) {
            object.parent = None;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_core::{Face, Object, Vec3};
    fn node(name: &str, parent: Option<&str>, transform: Mat4, geometry: bool) -> Object {
        Object {
            name: name.into(),
            parent: parent.map(str::to_owned),
            transform,
            visible: geometry,
            vertices: if geometry {
                vec![Vec3::ZERO, Vec3::X, Vec3::Y]
            } else {
                vec![]
            },
            faces: if geometry {
                vec![Face {
                    vertices: [0, 1, 2],
                    material: 0,
                    uv: [[0.; 2]; 3],
                    normals: [Vec3::Z; 3],
                }]
            } else {
                vec![]
            },
            material: 0,
            uv_channels: Default::default(),
        }
    }
    #[test]
    fn nested_empty_groups_flatten_once_and_keep_authored_geometry_parent_frames() {
        let a = Mat4::from_translation(Vec3::new(3., 0., 7.));
        let b = Mat4::from_rotation_y(0.7);
        let c = Mat4::from_translation(Vec3::X);
        let source = Model {
            objects: vec![
                node("wall", Some("inner"), c, true),
                node("inner", Some("outer"), b, false),
                node("outer", None, a, false),
                node("authored child", Some("wall"), c, true),
            ],
            ..Default::default()
        };
        let flat = display_groups(&source).unwrap();
        assert!(flat.objects[0].transform.abs_diff_eq(a * b * c, 0.00001));
        assert!(flat.objects[0].parent.is_none());
        assert_eq!(flat.objects[3].transform, c);
        let again = display_groups(&flat).unwrap();
        for (a, b) in flat.objects.iter().zip(&again.objects) {
            assert_eq!(a.transform, b.transform);
        }
        assert_eq!(flat.objects[0].vertices, source.objects[0].vertices);
        assert_eq!(source.objects[0].transform, c);
    }
    #[test]
    fn malformed_group_cycles_are_rejected() {
        let source = Model {
            objects: vec![
                node("a", Some("b"), Mat4::IDENTITY, false),
                node("b", Some("a"), Mat4::IDENTITY, false),
            ],
            ..Default::default()
        };
        assert!(display_groups(&source).is_err());
    }
}

/// Native angular dashpots, retained in the import report until a physics scene needs them.
#[derive(Clone, Debug)]
pub struct AngularDashpot {
    pub bodies: [String; 2],
    pub strength: f32,
    pub damping: f32,
}
pub fn angular_dashpots(report: &crate::ImportReport) -> Result<Vec<AngularDashpot>, String> {
    crate::binary::angular_dashpots(report)
}
