//! Camera-facing match markers using the original sprite and countdown artwork.
use super::*;
use bevy::{
    asset::RenderAssetUsages,
    light::NotShadowCaster,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use legacy_ui::{State, assets::LegacyAssets, canvas::Pixels};

#[derive(Component)]
pub(super) struct Marker(usize);
#[derive(Resource)]
pub(super) struct Frames(Vec<Handle<StandardMaterial>>);

const HEIGHT: f32 = 0.8;

pub(super) fn setup(
    mut commands: Commands,
    assets: Res<LegacyAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(title), Ok(countdown)) = (
        assets.image("common/immobile.tga"),
        assets.widget("common/immobility_countdown.gib"),
    ) else {
        return;
    };
    let mut state = State {
        root: Some(countdown),
        ..default()
    };
    let frames = (0..=10)
        .map(|seconds| {
            let mut pixels = Pixels::blank(104, 64);
            let bounds = pixels.bounds();
            pixels.image(title, title.bounds(), [1.; 4], true, bounds);
            if seconds > 0 {
                state
                    .values
                    .insert("immobile countdown".into(), seconds.to_string());
                let timer = legacy_ui::layout::compose(&assets, &state, 80, 32).pixels;
                pixels.image(&timer, Rect::new(12., 32., 92., 64.), [1.; 4], true, bounds);
            }
            let texture = images.add(Image::new(
                Extent3d {
                    width: 104,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                pixels.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            ));
            materials.add(StandardMaterial {
                base_color_texture: Some(texture),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            })
        })
        .collect::<Vec<_>>();
    let mesh = meshes.add(Rectangle::new(HEIGHT * 104. / 64., HEIGHT));
    for slot in 0..4 {
        commands.spawn((
            Marker(slot),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(frames[0].clone()),
            RenderLayers::layer(LAYER),
            NotShadowCaster,
            Visibility::Hidden,
        ));
    }
    commands.insert_resource(Frames(frames));
}

pub(super) fn update(
    session: Res<Session>,
    nav: Res<Navigation>,
    frames: Option<Res<Frames>>,
    camera: Query<&Transform, (With<MatchCamera>, Without<Marker>)>,
    bodies: Query<(&Transform, &Collider), (With<RobotBody>, Without<Marker>)>,
    mut markers: Query<(
        &Marker,
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let Some(frames) = frames else { return };
    for (marker, mut pose, mut visible, mut material) in &mut markers {
        let slot = marker.0;
        let seconds = session.immobility_countdown[slot] as usize;
        if nav.page != Page::Match
            || session.phase != Phase::Playing
            || (seconds == 0 && !session.counted_out[slot])
        {
            *visible = Visibility::Hidden;
            continue;
        }
        let robot = session.robots.iter().find(|r| r.id as usize == slot + 1);
        let Some((robot, chassis)) =
            robot.and_then(|r| Some((r, bodies.get(*r.bodies.first()?).ok()?.0)))
        else {
            *visible = Visibility::Hidden;
            continue;
        };
        let Ok(camera) = camera.get(session.camera) else {
            *visible = Visibility::Hidden;
            continue;
        };
        let top = robot
            .bodies
            .iter()
            .filter_map(|&body| bodies.get(body).ok())
            .map(|(t, c)| {
                c.raw
                    .compute_aabb(&bevy_rapier3d::rapier::math::Pose::from_parts(
                        t.translation,
                        t.rotation,
                    ))
                    .maxs
                    .y
            })
            .fold(chassis.translation.y, f32::max);
        pose.rotation = camera.rotation;
        pose.translation = chassis.translation.with_y(top + 0.12) + camera.up() * (HEIGHT * 0.5);
        material.0 = frames.0[seconds].clone();
        *visible = Visibility::Visible;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_tracks_robot_and_camera_until_motion_resumes() {
        let mut app = App::new();
        let camera = app
            .world_mut()
            .spawn((MatchCamera, Transform::default()))
            .id();
        let body = app
            .world_mut()
            .spawn((
                RobotBody,
                Collider::cuboid(1., 0.5, 1.),
                Transform::default(),
            ))
            .id();
        let marker = app
            .world_mut()
            .spawn((
                Marker(1),
                Transform::default(),
                Visibility::Hidden,
                MeshMaterial3d::<StandardMaterial>::default(),
            ))
            .id();
        let mut robot = RobotInstance::new(2, Entity::PLACEHOLDER, default());
        robot.bodies.push(body);
        app.insert_resource(Session {
            camera,
            robots: vec![robot],
            phase: Phase::Playing,
            immobility_countdown: [0, 10, 0, 0],
            ..default()
        })
        .insert_resource(Frames(vec![default(); 11]))
        .insert_resource(Navigation {
            page: Page::Match,
            back: Page::Main,
        })
        .add_systems(Update, update);
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(marker),
            Some(&Visibility::Visible)
        );
        assert!(app.world().get::<Transform>(marker).unwrap().translation.y > 0.5);
        let rotation = Quat::from_rotation_y(1.);
        app.world_mut()
            .get_mut::<Transform>(camera)
            .unwrap()
            .rotation = rotation;
        app.world_mut()
            .get_mut::<Transform>(body)
            .unwrap()
            .translation
            .x = 3.;
        app.update();
        let pose = app.world().get::<Transform>(marker).unwrap();
        assert_eq!(pose.translation.x, 3.);
        assert_eq!(pose.rotation, rotation);
        app.world_mut()
            .resource_mut::<Session>()
            .immobility_countdown[1] = 0;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(marker),
            Some(&Visibility::Hidden)
        );
    }
}
