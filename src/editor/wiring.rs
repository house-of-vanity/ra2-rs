//! Native workshop pointers, reconstructed from RA2's selected-control handler.
//! 0x443cb0 collects the controlled components; 0x443fc0 builds green four-point
//! branches and 0x4440d0 builds the blue control-board stem. No BOT data is changed.
use super::{Editor, Mode};
use arena_core::{
    Model,
    robot::{RobotAssembly, RobotDefinition},
};
use bevy::{camera::visibility::RenderLayers, prelude::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const BOARD_HEIGHT: f32 = 1.6; // Original 0x6a6268.
const BUS_HEIGHT: f32 = 1.0; // Original 0x6a626c.
const LINE_WIDTH: f32 = 4.0; // Original 0x6a6270, in render-target pixels.

#[derive(Default, Debug)]
struct Routes {
    board: Option<[Vec3; 2]>,
    branches: Vec<[Vec3; 4]>,
}

fn routes(
    robot: &RobotDefinition,
    assembly: &RobotAssembly,
    models: &BTreeMap<String, Arc<Model>>,
    selected: Option<usize>,
) -> Routes {
    let Some(control) = selected.and_then(|i| robot.controllers.get(i)) else {
        return Routes::default();
    };
    let anchor = |component| {
        let part = assembly
            .parts
            .iter()
            .find(|p| p.component == component && p.visible)?;
        let model = models.get(&part.model)?;
        // The native component bounds use a centre plus bounding radius. Restrict
        // this to display geometry: hidden collision proxies must not move a pointer.
        let mut points = model
            .objects
            .iter()
            .filter(|o| o.visible)
            .flat_map(|o| o.positions());
        let first = points.next()?;
        let (min, max) = points.fold((first, first), |(a, b), p| (a.min(p), b.max(p)));
        let centre = part.transform.transform_point3((min + max) * 0.5);
        let radius = (max - min).length() * 0.5;
        Some(centre + Vec3::Y * radius)
    };
    let Some(board) = robot
        .components
        .iter()
        .enumerate()
        .filter(|(_, p)| p.class == "ControlBoard")
        .find_map(|(i, _)| anchor(i))
    else {
        return Routes::default();
    };
    let bus = Vec3::new(board.x, BUS_HEIGHT, board.z);
    // BOT bindings identify the controller by its grid coordinates, not its index.
    // Analog inputs can bind both directions of one motor: draw one branch per part.
    let targets: BTreeSet<_> = robot
        .bindings
        .iter()
        .filter(|b| {
            b.controller == control.position
                && b.key < control.keys.len()
                && !b.action.is_empty()
                && b.action != "None"
        })
        .map(|b| b.component)
        .filter(|&i| {
            robot
                .components
                .get(i)
                .is_some_and(|p| p.class != "ControlBoard")
        })
        .collect();
    Routes {
        board: Some([board, bus]),
        branches: targets
            .into_iter()
            .filter_map(|i| {
                let end = anchor(i)?;
                Some([
                    Vec3::new(board.x, BOARD_HEIGHT, board.z),
                    bus,
                    Vec3::new(end.x, BUS_HEIGHT, end.z),
                    end,
                ])
            })
            .collect(),
    }
}

#[derive(Default)]
pub struct Overlay {
    root: Option<Entity>,
    meshes: Option<[Handle<Mesh>; 2]>,
    stamp: Option<(u64, Option<usize>, Option<Entity>, bool)>,
    camera: Option<Mat4>,
    routes: Routes,
}

impl Overlay {
    pub fn hide(&self, commands: &mut Commands) {
        if let Some(root) = self.root {
            commands.entity(root).insert(Visibility::Hidden);
        }
    }

    pub fn connection_count(&self) -> usize {
        self.routes.branches.len()
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Cached workshop overlay and render assets"
    )]
    pub fn update(
        &mut self,
        commands: &mut Commands,
        editor: &Editor,
        assembly: Option<&RobotAssembly>,
        models: &BTreeMap<String, Arc<Model>>,
        camera: Transform,
        scene: Option<Entity>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        let enabled = editor.mode == Mode::Wiring;
        let stamp = (
            editor.document.revision,
            editor.selected_controller,
            scene,
            enabled,
        );
        let changed = self.stamp != Some(stamp);
        if changed {
            self.routes = assembly
                .filter(|_| enabled)
                .map(|a| {
                    routes(
                        &editor.document.robot,
                        a,
                        models,
                        editor.selected_controller,
                    )
                })
                .unwrap_or_default();
            self.stamp = Some(stamp);
        }
        if !enabled || self.routes.board.is_none() {
            self.hide(commands);
            return;
        }
        if self.meshes.is_none() {
            let root = commands
                .spawn((
                    Name::new("Selected control wiring"),
                    Transform::default(),
                    Visibility::default(),
                ))
                .id();
            let handles = [
                meshes.add(ribbon(&[], camera)),
                meshes.add(ribbon(&[], camera)),
            ];
            for (handle, color) in handles
                .iter()
                .zip([Color::srgb(0., 0., 1.), Color::srgb(0., 1., 0.)])
            {
                commands.spawn((
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: color,
                        unlit: true,
                        cull_mode: None,
                        ..default()
                    })),
                    Transform::default(),
                    RenderLayers::layer(11),
                    bevy::camera::visibility::NoFrustumCulling,
                    ChildOf(root),
                ));
            }
            self.root = Some(root);
            self.meshes = Some(handles);
        }
        commands
            .entity(self.root.unwrap())
            .insert(Visibility::Visible);
        if changed || self.camera != Some(camera.to_matrix()) {
            let board = self.routes.board.unwrap();
            let branches: Vec<_> = self
                .routes
                .branches
                .iter()
                .flat_map(|p| p.windows(2).map(|w| [w[0], w[1]]))
                .collect();
            for (handle, segments) in self
                .meshes
                .as_ref()
                .unwrap()
                .iter()
                .zip([&[board][..], branches.as_slice()])
            {
                if let Some(mut mesh) = meshes.get_mut(handle) {
                    *mesh = ribbon(segments, camera);
                }
            }
            self.camera = Some(camera.to_matrix());
        }
    }
}

// Bevy's line primitive is one pixel wide. Camera-facing quads reproduce the
// original thick, unlit pointers with two reusable meshes and no extra textures.
// Only camera/selection/document changes rebuild these few vertices.
fn ribbon(segments: &[[Vec3; 2]], camera: Transform) -> Mesh {
    let inverse = camera.to_matrix().inverse();
    let tangent = (std::f32::consts::FRAC_PI_4 * 0.5).tan();
    let mut positions = Vec::with_capacity(segments.len() * 6);
    for segment in segments {
        let [mut a, mut b] = segment.map(|p| inverse.transform_point3(p));
        const NEAR: f32 = -0.02;
        if a.z >= NEAR && b.z >= NEAR {
            continue;
        }
        if a.z >= NEAR {
            a = a.lerp(b, (NEAR - a.z) / (b.z - a.z));
        }
        if b.z >= NEAR {
            b = b.lerp(a, (NEAR - b.z) / (a.z - b.z));
        }
        let delta = b.truncate() / -b.z - a.truncate() / -a.z;
        let Some(direction) = delta.try_normalize() else {
            continue;
        };
        let normal = Vec3::new(-direction.y, direction.x, 0.) * LINE_WIDTH * tangent / 512.;
        let [a0, a1, b0, b1] = [
            a + normal * -a.z,
            a - normal * -a.z,
            b + normal * -b.z,
            b - normal * -b.z,
        ]
        .map(|p| camera.to_matrix().transform_point3(p).to_array());
        positions.extend([a0, a1, b0, b0, a1, b1]);
    }
    // Keep an allocated, invisible primitive when a selected control has no
    // targets. Bevy otherwise warns on each empty-mesh upload.
    if positions.is_empty() {
        positions.resize(3, [0.; 3]);
    }
    let count = positions.len();
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0., 0.]; count])
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_core::{
        Object,
        editor::Workshop,
        robot::{AssemblyPart, ControlBinding, Controller, MountPlacement, RobotComponent},
    };

    fn fixture() -> (Workshop, RobotAssembly, BTreeMap<String, Arc<Model>>) {
        let mut document = Workshop::new();
        let robot = &mut document.robot;
        robot.chassis.bottom = arena_core::editor::chassis::rectangle(600., 600.);
        robot.chassis.top = robot.chassis.bottom.clone();
        arena_core::editor::chassis::rebuild(&mut robot.chassis).unwrap();
        robot.components = ["Chassis", "ControlBoard", "SpinMotor", "BurstMotor"]
            .into_iter()
            .map(|class| RobotComponent {
                class: class.into(),
                definition: String::new(),
                parent: Some(0),
                parent_mount: 0,
                own_mount: 0,
                mounting: MountPlacement::default(),
                settings: BTreeMap::from([
                    ("start_angle".into(), "0".into()),
                    ("stop_angle".into(), "1".into()),
                ]),
            })
            .collect();
        robot.components[0].parent = None;
        robot.controllers = [[2, 1], [0, 0]]
            .into_iter()
            .map(|position| Controller {
                name: String::new(),
                position,
                kind: 1,
                keys: vec![vec![0, 0, 17, 0], vec![0, 0, 31, 0]],
            })
            .collect();
        let binding = |component, controller, key, action: &str| ControlBinding {
            component,
            controller,
            key,
            action: action.into(),
        };
        robot.bindings = vec![
            binding(2, [2, 1], 0, "Spin Clockwise"),
            binding(2, [2, 1], 1, "Spin Counter-clockwise"),
            binding(3, [0, 0], 0, "Fire"),
            binding(3, [2, 1], 0, "None"),
            binding(3, [2, 1], 8, "Fire"),
            binding(99, [2, 1], 0, "Fire"),
        ];
        let object = Object {
            name: "display".into(),
            visible: true,
            parent: None,
            transform: Mat4::from_translation(Vec3::X * 0.2),
            vertices: vec![Vec3::splat(-0.1), Vec3::splat(0.1)],
            faces: vec![],
            material: 0,
            uv_channels: BTreeMap::new(),
        };
        let mut hidden = object.clone();
        hidden.visible = false;
        hidden.vertices = vec![Vec3::splat(100.)];
        let model = Arc::new(Model {
            objects: vec![object, hidden],
            ..default()
        });
        let assembly = RobotAssembly {
            name: String::new(),
            mass: 0.,
            bounds: (Vec3::ZERO, Vec3::ONE),
            parts: (0..4)
                .map(|component| AssemblyPart {
                    component,
                    model: "posed".into(),
                    mass: 0.,
                    visible: true,
                    transform: Mat4::from_rotation_translation(
                        Quat::from_rotation_y(0.7),
                        Vec3::Z * component as f32 * 0.2,
                    ),
                })
                .collect(),
        };
        (
            document,
            assembly,
            BTreeMap::from([("posed".into(), model)]),
        )
    }

    #[test]
    fn selected_control_uses_grid_identity_and_deduplicates_analog_targets() {
        let (document, assembly, models) = fixture();
        let r = routes(&document.robot, &assembly, &models, Some(0));
        assert_eq!(r.branches.len(), 1);
        let expected = assembly.parts[2].transform.transform_point3(Vec3::X * 0.2)
            + Vec3::Y * Vec3::splat(0.1).length();
        assert!(r.branches[0][3].abs_diff_eq(expected, 1e-6));
        assert_eq!(r.branches[0][0].y, 1.6);
        assert_eq!(r.branches[0][1].y, 1.0);
        assert_eq!(r.branches[0][2].y, 1.0);
        let other = routes(&document.robot, &assembly, &models, Some(1));
        assert_eq!(other.branches.len(), 1);
        assert_ne!(r.branches[0][3], other.branches[0][3]);
        assert!(
            routes(&document.robot, &assembly, &models, None)
                .board
                .is_none()
        );
        assert!(
            routes(&document.robot, &assembly, &models, Some(99))
                .board
                .is_none()
        );
    }

    #[test]
    fn removing_assignments_and_undoing_them_updates_routes_without_mutating_robot() {
        let (mut document, mut assembly, models) = fixture();
        document
            .robot
            .bindings
            .retain(|b| b.component < 4 && b.key < 2);
        let saved = ra2_formats::bot::encode(&document.robot).unwrap();
        routes(&document.robot, &assembly, &models, Some(0));
        assert_eq!(saved, ra2_formats::bot::encode(&document.robot).unwrap());
        document
            .edit("Unwire", |r| {
                r.bindings.clear();
                Ok(())
            })
            .unwrap();
        assert!(
            routes(&document.robot, &assembly, &models, Some(0))
                .branches
                .is_empty()
        );
        assert!(document.undo());
        assert_eq!(
            routes(&document.robot, &assembly, &models, Some(0))
                .branches
                .len(),
            1
        );
        assembly.parts[1].visible = false;
        assert!(
            routes(&document.robot, &assembly, &models, Some(0))
                .board
                .is_none()
        );
        assembly.parts[1].visible = true;
        assembly.parts[2].visible = false;
        assert!(
            routes(&document.robot, &assembly, &models, Some(0))
                .branches
                .is_empty()
        );
    }

    #[test]
    fn pointers_have_constant_screen_width_and_clip_behind_camera() {
        let camera = Transform::default();
        let mesh = ribbon(
            &[
                [Vec3::new(0., 0., -1.), Vec3::new(1., 0., -1.)],
                [Vec3::new(0., 0., -5.), Vec3::new(1., 0., -5.)],
                [Vec3::new(-1., 0., 0.), Vec3::new(1., 0., -1.)],
                [Vec3::ZERO, Vec3::ONE],
            ],
            camera,
        );
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(vertices)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("Missing positions");
        };
        assert_eq!(vertices.len(), 18);
        assert!(vertices.iter().flatten().all(|v| v.is_finite()));
        let pixels = |a: [f32; 3], b: [f32; 3]| {
            ((Vec3::from(a).truncate() / -a[2]) - (Vec3::from(b).truncate() / -b[2])).length()
                * 512.
                / (2. * (std::f32::consts::FRAC_PI_4 * 0.5).tan())
        };
        assert!((pixels(vertices[0], vertices[1]) - 4.).abs() < 1e-5);
        assert!((pixels(vertices[6], vertices[7]) - 4.).abs() < 1e-5);
    }
}
