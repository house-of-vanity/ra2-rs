//! Offscreen GPU exercise of the actual legacy GUI input, HQ and workshop.
use crate::{
    editor::{Editor, Mode},
    frontend::{MenuCamera, Navigation, Page},
    legacy_ui::Output,
};
use bevy::{
    camera::RenderTarget,
    input::{ButtonState, mouse::MouseButtonInput},
    prelude::*,
};
#[derive(Resource)]
struct Capture(Handle<Image>);
#[derive(Default)]
struct Check {
    stage: usize,
    frames: usize,
    release: Option<Entity>,
    wired_component: Option<usize>,
    preview_pose: Vec<(Entity, Transform)>,
    wall_waits: usize,
    wall_skin: Option<(Handle<Mesh>, Vec<[f32; 3]>)>,
}
pub struct WorkshopCheckPlugin;
impl Plugin for WorkshopCheckPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup.after(crate::frontend::FrontendSetup))
            .add_systems(Update, run.after(crate::legacy_ui::Paint));
    }
}
fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    camera: Single<Entity, With<MenuCamera>>,
) {
    let image = images.add(Image::new_target_texture(
        800,
        600,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    commands
        .entity(*camera)
        .insert(RenderTarget::Image(image.clone().into()));
    commands.insert_resource(Capture(image));
}
fn click(world: &mut World, name: &str) -> Entity {
    let point = {
        let output = world.resource::<Output>();
        let hit = output
            .hits
            .iter()
            .rev()
            .find(|h| h.name == name)
            .unwrap_or_else(|| panic!("Missing original GUI control {name}"));
        let rect = hit.rect.intersect(hit.clip);
        assert!(!rect.is_empty(), "Clipped GUI control {name}");
        output.origin + rect.center() * output.scale
    };
    pointer(world, point)
}
fn pointer(world: &mut World, point: Vec2) -> Entity {
    let mut windows = world.query::<(Entity, &mut Window)>();
    let (entity, mut window) = windows.single_mut(world).unwrap();
    window.set_cursor_position(Some(point));
    world.write_message(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window: entity,
    });
    entity
}
fn hover(world: &mut World, point: Vec2) {
    let mut windows = world.query::<&mut Window>();
    windows
        .single_mut(world)
        .unwrap()
        .set_cursor_position(Some(point));
}
fn placement_point(world: &World) -> Vec2 {
    let view = world.resource::<crate::editor::viewport::View>();
    let output = world.resource::<Output>();
    let rect = output.regions["3d component placeholder"];
    let point = view
        .pose
        .to_matrix()
        .inverse()
        .transform_point3(Vec3::new(-0.25, 0., 0.));
    let tangent = (std::f32::consts::FRAC_PI_4 * 0.5).tan();
    let uv = Vec2::new(
        0.5 + point.x / (-point.z) / (2. * tangent),
        0.5 - point.y / (-point.z) / (2. * tangent),
    );
    output.origin + (rect.min + uv * rect.size()) * output.scale
}
fn key(world: &mut World, state: ButtonState) {
    native_key(world, KeyCode::KeyK, state);
}
fn native_key(world: &mut World, key_code: KeyCode, state: ButtonState) {
    let window = world
        .query_filtered::<Entity, With<Window>>()
        .single(world)
        .unwrap();
    world.write_message(bevy::input::keyboard::KeyboardInput {
        key_code,
        logical_key: match key_code {
            KeyCode::Space => bevy::input::keyboard::Key::Space,
            KeyCode::KeyW => bevy::input::keyboard::Key::Character("w".into()),
            _ => bevy::input::keyboard::Key::Character("k".into()),
        },
        state,
        text: (key_code == KeyCode::KeyK).then(|| "k".into()),
        repeat: false,
        window,
    });
}
fn wiring_motor_point(world: &World, component: Option<usize>) -> (Vec2, usize) {
    component_point(world, component, "SpinMotor", "3d wiring placeholder")
}
fn slider(world: &mut World, name: &str, ratio: f32) -> Entity {
    let output = world.resource::<Output>();
    let hit = output.hits.iter().find(|h| h.name == name).unwrap();
    let point = output.origin
        + Vec2::new(
            hit.rect.min.x + hit.rect.width() * ratio,
            hit.rect.center().y,
        ) * output.scale;
    pointer(world, point)
}
fn component_point(
    world: &World,
    component: Option<usize>,
    class: &str,
    region: &str,
) -> (Vec2, usize) {
    let view = world.resource::<crate::editor::viewport::View>();
    let output = world.resource::<Output>();
    let editor = world.resource::<Editor>();
    let rect = output.regions[region];
    for y in (5..rect.height() as usize).step_by(8) {
        for x in (5..rect.width() as usize).step_by(8) {
            let point = rect.min + Vec2::new(x as f32, y as f32);
            if let Some(pick) = view
                .pick(point, rect, true)
                .or_else(|| view.pick(point, rect, false))
                && editor.document.robot.components[pick.component].class == class
                && component.is_none_or(|c| c == pick.component)
            {
                return (output.origin + point * output.scale, pick.component);
            }
        }
    }
    panic!("No visible motor in the native wiring viewport");
}
fn screenshot(world: &mut World, name: &str) {
    let image = world.resource::<Capture>().0.clone();
    world
        .spawn(bevy::render::view::screenshot::Screenshot::image(image))
        .observe(bevy::render::view::screenshot::save_to_disk(format!(
            "logs/hq-{name}.png"
        )));
}
fn run(world: &mut World, mut check: Local<Check>) {
    check.frames += 1;
    if check.frames < 12 {
        return;
    }
    check.frames = 0;
    if let Some(window) = check.release.take() {
        world.write_message(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Released,
            window,
        });
        return;
    }
    match check.stage {
        0 => {
            world.resource_mut::<Navigation>().page = Page::TeamSelect;
        }
        1 => screenshot(world, "select"),
        2 => {
            check.release = Some(click(world, "continue button"));
        }
        3 => {
            assert_eq!(world.resource::<Navigation>().page, Page::TeamHQ);
            screenshot(world, "inventory");
        }
        4 => {
            check.release = Some(click(world, "edit team button"));
        }
        5 => {
            screenshot(world, "edit-team");
            check.release = Some(click(world, "Down"));
        }
        6 => {
            assert_eq!(
                world
                    .resource::<crate::legacy_ui::State>()
                    .scroll
                    .get("logo listbox"),
                Some(&1)
            );
            check.release = Some(click(world, "cancel button"));
        }
        7 => {
            check.release = Some(click(world, "robot:0"));
        }
        8 => {
            check.release = Some(click(world, "workshop button"));
        }
        9 => {
            assert_eq!(world.resource::<Navigation>().page, Page::Workshop);
            screenshot(world, "workshop");
        }
        10 => {
            check.release = Some(click(world, "components button"));
        }
        11 => screenshot(world, "components"),
        12 => {
            check.release = Some(click(world, "back button"));
        }
        13 => {
            assert_eq!(world.resource::<Navigation>().page, Page::TeamHQ);
            check.release = Some(click(world, "select team button"));
        }
        14 => {
            check.release = Some(click(world, "new team button"));
        }
        15 => screenshot(world, "new-team"),
        16 => {
            check.release = Some(click(world, "cancel button"));
        }
        17 => {
            // Fresh unsaved document: this check never saves into native player files.
            let revision = world.resource::<Editor>().document.revision + 1;
            let mut editor = world.resource_mut::<Editor>();
            *editor = Editor::default();
            editor.document.revision = revision;
            world.resource_mut::<Navigation>().page = Page::Workshop;
        }
        18 => {
            check.release = Some(click(world, "chassis button"));
        }
        19 => {
            check.release = Some(click(world, "structure design button"));
        }
        20 => {
            assert_eq!(world.resource::<Editor>().mode, Mode::Base);
            screenshot(world, "freeform-grid");
        }
        21..=26 => {
            let (x, y) = [
                (0.2, 0.2),
                (0.8, 0.2),
                (0.55, 0.5),
                (0.8, 0.8),
                (0.2, 0.8),
                (0.2, 0.2),
            ][check.stage - 21];
            let output = world.resource::<Output>();
            let rect = output.regions["Editing Area"];
            let point = output.origin + (rect.min + Vec2::new(x, y) * rect.size()) * output.scale;
            check.release = Some(pointer(world, point));
        }
        27 => {
            let editor = world.resource::<Editor>();
            assert_eq!(
                editor.outline.len(),
                5,
                "Pointer clicks did not plot a concave freeform outline"
            );
            assert!(editor.closed);
            screenshot(world, "freeform-closed");
        }
        28 => {
            check.release = Some(click(world, "step 2 button"));
        }
        29 => {
            assert!(
                world
                    .resource::<Editor>()
                    .document
                    .robot
                    .chassis
                    .model
                    .is_some()
            );
            screenshot(world, "topplate");
        }
        30 => {
            check.release = Some(click(world, "finished button"));
        }
        31 => {
            check.release = Some(click(world, "components button"));
        }
        32 => {
            let key = world
                .resource::<crate::editor::catalog::Catalog>()
                .browser("power")[0]
                .key
                .clone();
            check.release = Some(click(world, &format!("component:{key}")));
        }
        33 => {
            check.release = Some(click(world, "attach button"));
        }
        34 => {
            let output = world.resource::<Output>();
            let rect = output.regions["3d component placeholder"];
            let point = output.origin + (rect.min + rect.size() * Vec2::splat(0.05)) * output.scale;
            hover(world, point);
        }
        35 => {
            let editor = world.resource::<Editor>();
            assert!(
                editor.pending.is_some() && editor.ghost_pose.is_some() && !editor.ghost_valid,
                "Invalid placement disappeared or was marked valid"
            );
            screenshot(world, "invalid-placement");
        }
        36 => {
            let point = placement_point(world);
            hover(world, point);
        }
        37 => {
            let editor = world.resource::<Editor>();
            assert!(
                editor.ghost_valid && editor.ghost_pose.is_some(),
                "Clear baseplate placement was rejected"
            );
            screenshot(world, "valid-placement");
        }
        38 => {
            let point = placement_point(world);
            check.release = Some(pointer(world, point));
        }
        39 => {
            let editor = world.resource::<Editor>();
            assert_eq!(editor.document.robot.components.len(), 2);
            assert!(editor.pending.is_none());
            screenshot(world, "attached-component");
        }
        40 => {
            check.release = Some(click(world, "back button"));
        }
        41 => {
            assert!(matches!(
                world.resource::<Editor>().dialog,
                Some(crate::editor::Dialog::SavePrompt)
            ));
            screenshot(world, "save-prompt");
            check.release = Some(click(world, "cancel button"));
        }
        42 => {
            assert_eq!(world.resource::<Navigation>().page, Page::Workshop);
            assert!(world.resource::<Editor>().document.dirty);
            world.resource_mut::<Navigation>().page = Page::TeamHQ;
        }
        43 => check.release = Some(click(world, "league button")),
        44 => {
            screenshot(world, "league");
            check.release = Some(click(world, "Down"));
        }
        45 => {
            assert_eq!(
                world
                    .resource::<crate::legacy_ui::State>()
                    .scroll
                    .get("league grid"),
                Some(&1)
            );
            screenshot(world, "league-scrolled");
            check.release = Some(click(world, "history button"));
        }
        46 => screenshot(world, "history"),
        47 => check.release = Some(click(world, "events button")),
        48 => screenshot(world, "events"),
        49 => check.release = Some(click(world, "event:0")),
        50 => {
            screenshot(world, "event-details");
            check.release = Some(click(world, "cancel button"));
        }
        51 => {
            let path = world
                .resource::<crate::Options>()
                .game_dir
                .join("Teams/Team0/Bot0.bot");
            let mut editor = world.resource_mut::<Editor>();
            crate::editor::controls::load(&mut editor, &path).unwrap();
            editor
                .document
                .edit("Wiring check fixture", |r| {
                    r.controllers.clear();
                    r.bindings.clear();
                    Ok(())
                })
                .unwrap();
            editor.team_slot = None;
            editor.selected_controller = None;
            editor.selected_control_slot = [0, 0];
            editor.mode = Mode::Wiring;
            editor.dialog = None;
            world.resource_mut::<Navigation>().page = Page::Workshop;
        }
        52 => check.release = Some(click(world, "new button")),
        53 => check.release = Some(click(world, "input1")),
        54 => key(world, ButtonState::Pressed),
        55 => {
            key(world, ButtonState::Released);
            check.release = Some(click(world, "ok button"));
        }
        56 => {
            assert_eq!(
                world.resource::<Editor>().document.robot.controllers[0].keys[0][2],
                37
            );
            let (point, index) = wiring_motor_point(world, None);
            check.wired_component = Some(index);
            check.release = Some(pointer(world, point));
        }
        57 => check.release = Some(click(world, "input1 map")),
        58 => check.release = Some(click(world, "choice:input1 map:1")),
        59 => {
            assert_eq!(world.resource::<Editor>().binding_actions, [1, 0]);
            check.release = Some(click(world, "ok button"));
        }
        60 => {
            assert_eq!(world.resource::<Editor>().document.robot.bindings.len(), 1);
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                1
            );
            screenshot(world, "wiring-saved");
            let (point, _) = wiring_motor_point(world, check.wired_component);
            check.release = Some(pointer(world, point));
        }
        61 => {
            assert_eq!(
                world.resource::<Editor>().binding_actions,
                [1, 0],
                "Reopening cleared motor wiring"
            );
            assert_eq!(
                world.resource::<crate::legacy_ui::State>().values["input1 map"],
                "Spin Clockwise"
            );
            screenshot(world, "wiring-reopened");
            check.release = Some(click(world, "ok button"));
        }
        62 => {
            let mut editor = world.resource_mut::<Editor>();
            assert_eq!(
                editor.document.robot.bindings.len(),
                1,
                "Confirming removed wiring"
            );
            crate::editor::controls::save(
                &mut editor,
                std::path::Path::new("logs/wiring-check.bot"),
            )
            .unwrap();
            crate::editor::controls::load(
                &mut editor,
                std::path::Path::new("logs/wiring-check.bot"),
            )
            .unwrap();
            editor.selected_controller = Some(0);
        }
        63 => {
            let (point, _) = wiring_motor_point(world, check.wired_component);
            check.release = Some(pointer(world, point));
        }
        64 => {
            assert_eq!(
                world.resource::<Editor>().binding_actions,
                [1, 0],
                "BOT reload cleared wiring"
            );
            screenshot(world, "wiring-reloaded");
            check.release = Some(click(world, "input1 map"));
        }
        65 => check.release = Some(click(world, "choice:input1 map:0")),
        66 => check.release = Some(click(world, "cancel button")),
        67 => {
            assert_eq!(
                world.resource::<Editor>().document.robot.bindings.len(),
                1,
                "Cancel changed wiring"
            );
            let (point, _) = wiring_motor_point(world, check.wired_component);
            check.release = Some(pointer(world, point));
        }
        68 => {
            assert_eq!(world.resource::<Editor>().binding_actions, [1, 0]);
            check.release = Some(click(world, "ok button"));
        }
        69 => {
            assert_eq!(world.resource::<Editor>().document.robot.bindings.len(), 1);
            let path = world
                .resource::<crate::Options>()
                .game_dir
                .join("Teams/Team0/Bot0.bot");
            let mut editor = world.resource_mut::<Editor>();
            crate::editor::controls::load(&mut editor, &path).unwrap();
            editor.mode = Mode::Overview;
        }
        70 => screenshot(world, "crush-overview"),
        71 => check.release = Some(click(world, "components button")),
        72 => {
            let (point, _) = component_point(world, None, "BurstMotor", "3d component placeholder");
            check.release = Some(pointer(world, point));
        }
        73 => {
            assert!(
                world
                    .resource::<Output>()
                    .hits
                    .iter()
                    .any(|h| h.name == "start angle slider")
            );
            screenshot(world, "crush-adjustments");
        }
        74 => check.release = Some(slider(world, "start angle slider", 0.62)),
        75 => {
            let editor = world.resource::<Editor>();
            let index = editor.selected.unwrap();
            let start: f32 = editor.document.robot.components[index].settings["start_angle"]
                .parse()
                .unwrap();
            assert!((start - 0.62 * std::f32::consts::TAU).abs() < 0.001);
            assert!(!editor.preview_burst_end);
            screenshot(world, "crush-start-angle");
        }
        76 => check.release = Some(slider(world, "end angle slider", 0.25)),
        77 => {
            let editor = world.resource::<Editor>();
            let index = editor.selected.unwrap();
            let stroke: f32 = editor.document.robot.components[index].settings["stop_angle"]
                .parse()
                .unwrap();
            assert!((stroke + 1.2).abs() < 0.001);
            assert!(editor.preview_burst_end);
            let mut displayed = editor.document.robot.clone();
            let part = &mut displayed.components[index];
            let start: f32 = part.settings["start_angle"].parse().unwrap();
            part.settings
                .insert("start_angle".into(), (start + stroke).to_string());
            let expected = world
                .resource::<crate::editor::catalog::Catalog>()
                .freeze(&displayed)
                .unwrap();
            let view = world.resource::<crate::editor::viewport::View>();
            for (a, b) in view
                .assembly
                .as_ref()
                .unwrap()
                .parts
                .iter()
                .zip(&expected.assembly.parts)
            {
                assert!(
                    a.transform.abs_diff_eq(b.transform, 0.0001),
                    "End-angle preview did not update the robot"
                );
            }
            screenshot(world, "crush-end-angle");
        }
        78 => check.release = Some(click(world, "overview button")),
        79 => {
            let mut editor = world.resource_mut::<Editor>();
            crate::editor::controls::save(
                &mut editor,
                std::path::Path::new("logs/pose-range-check.bot"),
            )
            .unwrap();
            let decoded =
                ra2_formats::bot::decode(&std::fs::read("logs/pose-range-check.bot").unwrap())
                    .unwrap();
            assert!(
                decoded
                    .components
                    .iter()
                    .filter(|p| p.class == "BurstMotor")
                    .any(
                        |p| (p.settings["stop_angle"].parse::<f32>().unwrap() + 1.2).abs() < 0.001
                    )
            );
            screenshot(world, "crush-adjusted-overview");
        }
        80 => check.release = Some(click(world, "wiring button")),
        81 => check.release = Some(click(world, "control:1")),
        82 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                4
            );
            screenshot(world, "wiring-drive-pointers");
            check.release = Some(click(world, "control:0"));
        }
        83 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                1
            );
            screenshot(world, "wiring-fire-pointers");
            world.resource_mut::<crate::editor::viewport::View>().yaw += 0.7;
        }
        84 => {
            screenshot(world, "wiring-rotated-pointers");
            check.release = Some(click(world, "control:11"));
        }
        85 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                0
            );
            screenshot(world, "wiring-empty-control");
            check.release = Some(click(world, "control:0"));
        }
        86 => {
            let (point, _) = component_point(world, None, "BurstMotor", "3d wiring placeholder");
            check.release = Some(pointer(world, point));
        }
        87 => check.release = Some(click(world, "input1 map")),
        88 => check.release = Some(click(world, "choice:input1 map:0")),
        89 => check.release = Some(click(world, "ok button")),
        90 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                0
            );
            screenshot(world, "wiring-unwired-pointers");
            assert!(world.resource_mut::<Editor>().document.undo());
        }
        91 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                1
            );
            screenshot(world, "wiring-undo-pointers");
            check.release = Some(click(world, "delete button"));
        }
        92 => {
            assert!(world.resource::<Editor>().selected_controller.is_none());
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                0
            );
            screenshot(world, "wiring-deleted-control");
            let mut editor = world.resource_mut::<Editor>();
            assert!(editor.document.undo());
            editor.selected_controller = editor
                .document
                .robot
                .controllers
                .iter()
                .position(|c| c.position == [0, 0]);
        }
        93 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                1
            );
            let mut editor = world.resource_mut::<Editor>();
            crate::editor::controls::save(
                &mut editor,
                std::path::Path::new("logs/wiring-pointers-check.bot"),
            )
            .unwrap();
            crate::editor::controls::load(
                &mut editor,
                std::path::Path::new("logs/wiring-pointers-check.bot"),
            )
            .unwrap();
            editor.selected_controller = editor
                .document
                .robot
                .controllers
                .iter()
                .position(|c| c.position == [0, 0]);
        }
        94 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                1
            );
            screenshot(world, "wiring-reloaded-pointers");
            check.release = Some(click(world, "overview button"));
        }
        95 => {
            assert_eq!(
                world
                    .resource::<crate::editor::viewport::View>()
                    .wiring
                    .connection_count(),
                0
            );
            let path = world
                .resource::<crate::Options>()
                .game_dir
                .join("Teams/Team0/Bot0.bot");
            crate::editor::controls::load(&mut world.resource_mut::<Editor>(), &path).unwrap();
            check.release = Some(click(world, "test robot button"));
        }
        96 | 97 | 101 | 102 => (),
        98 => {
            screenshot(world, "crush-visible-wheel-contact");
            check.release = Some(click(world, "components button"));
        }
        99 => {
            let path = world
                .resource::<crate::Options>()
                .game_dir
                .join("AI/Team0/Bot0.bot");
            crate::editor::controls::load(&mut world.resource_mut::<Editor>(), &path).unwrap();
        }
        100 => check.release = Some(click(world, "test robot button")),
        103 => {
            screenshot(world, "scout-visible-wheel-contact");
        }
        104 => check.release = Some(click(world, "wiring button")),
        105 => {
            let objects = world
                .resource::<crate::editor::viewport::View>()
                .preview_entities();
            check.preview_pose = objects
                .into_iter()
                .map(|e| (e, *world.get::<Transform>(e).unwrap()))
                .collect();
            native_key(world, KeyCode::KeyW, ButtonState::Pressed);
            native_key(world, KeyCode::Space, ButtonState::Pressed);
        }
        106 => {
            assert!(
                check.preview_pose.iter().any(|(e, t)| !world
                    .get::<Transform>(*e)
                    .unwrap()
                    .to_matrix()
                    .abs_diff_eq(t.to_matrix(), 0.001)),
                "Wiring key preview did not animate rendered geometry"
            );
            screenshot(world, "wiring-live-actions");
            native_key(world, KeyCode::KeyW, ButtonState::Released);
            native_key(world, KeyCode::Space, ButtonState::Released);
            check.release = Some(click(world, "components button"));
        }
        107 => {
            let catalog = world.resource::<crate::editor::catalog::Catalog>();
            let key = catalog
                .components
                .values()
                .find(|a| a.definition.value("dir") == "airtank")
                .or_else(|| {
                    catalog
                        .components
                        .values()
                        .find(|a| a.definition.value("base") == "AirTank")
                })
                .unwrap()
                .key
                .clone();
            let mut editor = world.resource_mut::<Editor>();
            editor.component = key;
            editor.selected_socket = 1;
            editor.ui_revision += 1;
        }
        108 => screenshot(world, "attachment-volume"),
        109 => check.release = Some(click(world, "attach button")),
        110 => {
            let (point, _) =
                component_point(world, None, "ControlBoard", "3d component placeholder");
            hover(world, point);
        }
        111 => {
            let editor = world.resource::<Editor>();
            let pick = editor
                .ghost_pick
                .as_ref()
                .expect("Placement lost its floor point");
            assert_eq!(
                pick.component, 0,
                "base-mounted tank snapped to a component body"
            );
            assert!(pick.attachment.is_none());
            assert!(editor.ghost_pose.is_some());
            screenshot(world, "baseplate-no-snap");
            check.release = Some(click(world, "test robot button"));
        }
        112 | 113 => (),
        114 => {
            let handle = world
                .query::<(&Name, &Mesh3d)>()
                .iter(world)
                .find(|(n, _)| n.as_str().starts_with("Chassis skin /"))
                .expect("Garage hull skin missing")
                .1
                .0
                .clone();
            let original = world
                .resource::<Assets<Mesh>>()
                .get(&handle)
                .unwrap()
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap()
                .to_vec();
            check.wall_skin = Some((handle, original));
            // Scout reverses into the near native garage wall under its own power.
            native_key(world, KeyCode::KeyS, ButtonState::Pressed);
        }
        115 => {
            check.wall_waits += 1;
            if world
                .resource::<crate::legacy_ui::State>()
                .floating_text
                .is_empty()
            {
                assert!(
                    check.wall_waits < 80,
                    "Driven robot never displayed native wall damage"
                );
                return;
            }
            native_key(world, KeyCode::KeyS, ButtonState::Released);
            let (handle, original) = check.wall_skin.as_ref().unwrap();
            let current = world
                .resource::<Assets<Mesh>>()
                .get(handle)
                .unwrap()
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap();
            assert_ne!(
                current, original,
                "Damage number appeared without a visible hull dent"
            );
            screenshot(world, "garage-native-hp");
        }
        116..=119 => (),
        120 => {
            screenshot(world, "garage-native-dent");
            check.release = Some(click(world, "reset button"));
        }
        121 | 122 => (),
        123 => {
            assert!(
                world
                    .resource::<crate::legacy_ui::State>()
                    .floating_text
                    .is_empty(),
                "RESET did not clear damage numbers"
            );
            check.release = Some(click(world, "overview button"));
        }
        124 => check.release = Some(slider(world, "heading slider", 0.25)),
        125 => {
            let heading = world.resource::<Editor>().document.robot.heading;
            assert!((heading - std::f32::consts::FRAC_PI_2).abs() < 0.001);
            let (_, pose, visible) = world
                .query::<(&Name, &Transform, &Visibility)>()
                .iter(world)
                .find(|(n, _, _)| n.as_str() == "Forward heading arrow")
                .expect("Native heading arrow missing");
            assert_eq!(*visible, Visibility::Visible);
            assert!((pose.rotation * Vec3::NEG_Z).abs_diff_eq(Vec3::X, 0.001));
            screenshot(world, "forward-heading-arrow");
            let path = std::path::Path::new("logs/forward-heading-check.bot");
            {
                let mut editor = world.resource_mut::<Editor>();
                crate::editor::controls::save(&mut editor, path).unwrap();
                crate::editor::controls::load(&mut editor, path).unwrap();
                assert!((editor.document.robot.heading - heading).abs() < 0.00001);
            }
            check.release = Some(click(world, "test robot button"));
        }
        126 => (),
        127 => {
            use bevy_rapier3d::prelude::RapierContextEntityLink;
            let context = world
                .query_filtered::<Entity, With<crate::garage::GarageContext>>()
                .single(world)
                .unwrap();
            let (_, pose, _) = world
                .query::<(&Name, &Transform, &RapierContextEntityLink)>()
                .iter(world)
                .find(|(n, _, c)| n.as_str() == "Chassis" && c.0 == context)
                .unwrap();
            assert!(
                (pose.rotation * Vec3::NEG_Z).dot(Vec3::NEG_X) > 0.98,
                "Garage ignored saved heading"
            );
            screenshot(world, "forward-heading-garage");
            check.release = Some(click(world, "overview button"));
        }
        128 => check.release = Some(slider(world, "heading slider", 1.)),
        129 => {
            assert!(
                (world.resource::<Editor>().document.robot.heading - std::f32::consts::TAU).abs()
                    < 0.001
            );
            let value = world.resource::<crate::legacy_ui::State>().values["heading slider"]
                .parse::<f32>()
                .unwrap();
            assert_eq!(value, 1., "360 degree thumb jumped back to zero");
            screenshot(world, "forward-heading-full-turn");
            check.release = Some(click(world, "components button"));
        }
        130 => {
            let (_, visible) = world
                .query::<(&Name, &Visibility)>()
                .iter(world)
                .find(|(n, _)| n.as_str() == "Forward heading arrow")
                .unwrap();
            assert_eq!(
                *visible,
                Visibility::Hidden,
                "Heading arrow leaked into Components"
            );
            println!(
                "PASS: native HQ/workshop and wiring; driven native wall HP/dents/RESET; native forward-heading arrow, slider endpoints, BOT reload and garage orientation"
            );
            world.write_message(crate::shutdown::ExitRequest);
        }
        _ => return,
    }
    check.stage += 1;
}
