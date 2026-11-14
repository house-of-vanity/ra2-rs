use super::{Editor, Mode, catalog::Catalog, placement};
use crate::{
    frontend::{Navigation, Page},
    legacy_ui,
};
use arena_core::{Model, robot::RobotAssembly};
use arena_runtime::{
    content::{FileTextures, TextureBytes, TextureSource},
    render::{self, PreparedModel, RenderSettings, TextureCache},
};
use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::render_resource::TextureFormat,
};
use ra2_formats::components::normalize;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Resource)]
pub struct View {
    pub pose: Transform,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub main: Entity,
    pub preview: Entity,
    main_overlay: Entity,
    preview_overlay: Entity,
    ghost_materials: Vec<Handle<StandardMaterial>>,
    ghost_state: Option<bool>,
    ghost_socket: u32,
    mount_root: Option<Entity>,
    mount_stamp: (u64, String, u32),
    mounts: Vec<Mount>,
    baseplate_texture: Option<Handle<Image>>,
    pub target: Handle<Image>,
    pub preview_target: Handle<Image>,
    pub assembly: Option<RobotAssembly>,
    pub wiring: super::wiring::Overlay,
    heading: super::heading::Indicator,
    pub(super) action_preview: Option<super::preview::Preview>,
    pub(super) models: BTreeMap<String, Arc<Model>>,
    pub(super) collisions: BTreeMap<String, Vec<(String, bevy_rapier3d::prelude::Collider)>>,
    preview_model: Option<Arc<Model>>,
    end_preview: Option<usize>,
    prepared: BTreeMap<String, PreparedModel>,
    root: Option<Entity>,
    preview_root: Option<Entity>,
    revision: u64,
    mode: Option<Mode>,
    component: String,
    cache: TextureCache,
    ghost_root: Option<Entity>,
    ghost_key: String,
    pub preview_yaw: f32,
    pub preview_pitch: f32,
    preview_model_pose: Mat4,
    preview_pose: Transform,
    socket_models: Option<(PreparedModel, PreparedModel)>,
    socket_bounds: Option<(Vec3, Vec3)>,
    socket_selected: u32,
}
#[derive(Clone, Debug)]
pub struct Pick {
    pub attachment: Option<u32>,
    pub component: usize,
    pub position: Vec3,
    pub normal: Vec3,
    pub face: usize,
    pub uv: Vec2,
}
struct Mount {
    pick: Pick,
    pose: Mat4,
    normal: Entity,
    selected: Entity,
}
struct PaintTexture<'a>(&'a [u8]);
impl TextureSource for PaintTexture<'_> {
    fn read(&self, _name: &str) -> Result<TextureBytes, String> {
        if self.0.len() < 18 {
            return Err("No chassis paint image".into());
        }
        let mut bytes = self.0.to_vec();
        for pixel in bytes[18..].as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        Ok(TextureBytes {
            key: "editor#paint".into(),
            extension: "tga".into(),
            bytes,
        })
    }
}
pub fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    options: Res<crate::Options>,
) {
    let target = images.add(Image::new_target_texture(
        512,
        512,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let preview_target = images.add(Image::new_target_texture(
        256,
        256,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let camera = |commands: &mut Commands, target: &Handle<Image>, layer| {
        commands
            .spawn((
                Camera3d::default(),
                Camera {
                    is_active: false,
                    order: -2,
                    clear_color: ClearColorConfig::Custom(Color::srgb(0.12, 0.13, 0.13)),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                RenderLayers::layer(layer),
                Tonemapping::None,
                bevy::core_pipeline::tonemapping::DebandDither::Disabled,
                Msaa::Sample4,
                Transform::default(),
            ))
            .id()
    };
    let main = camera(&mut commands, &target, 7);
    let preview = camera(&mut commands, &preview_target, 8);
    let main_overlay = camera(&mut commands, &target, 9);
    let preview_overlay = camera(&mut commands, &preview_target, 10);
    for overlay in [main_overlay, preview_overlay] {
        commands.entity(overlay).insert(Camera {
            is_active: false,
            order: -1,
            clear_color: ClearColorConfig::None,
            ..default()
        });
    }
    commands
        .entity(main_overlay)
        .insert(RenderLayers::from_layers(&[9, 11]));
    let baseplate_texture = std::fs::read(options.game_dir.join("UI/botlab/baseplate_grid.bmp"))
        .ok()
        .and_then(|bytes| {
            Image::from_buffer(
                &bytes,
                bevy::image::ImageType::Extension("bmp"),
                bevy::image::CompressedImageFormats::NONE,
                true,
                bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                    address_mode_u: bevy::image::ImageAddressMode::Repeat,
                    address_mode_v: bevy::image::ImageAddressMode::Repeat,
                    ..default()
                }),
                bevy::asset::RenderAssetUsages::default(),
            )
            .ok()
        })
        .map(|image| images.add(image));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.6, 0.0)),
        RenderLayers::from_layers(&[7, 8, 9]),
    ));
    commands.insert_resource(View {
        pose: Transform::default(),
        yaw: 0.65,
        pitch: 0.5,
        distance: 3.8,
        main,
        preview,
        main_overlay,
        preview_overlay,
        ghost_materials: vec![],
        ghost_state: None,
        ghost_socket: 0,
        mount_root: None,
        mount_stamp: (u64::MAX, String::new(), 0),
        mounts: vec![],
        baseplate_texture,
        target,
        preview_target,
        assembly: None,
        wiring: super::wiring::Overlay::default(),
        heading: super::heading::Indicator::default(),
        action_preview: None,
        models: BTreeMap::new(),
        collisions: BTreeMap::new(),
        preview_model: None,
        end_preview: None,
        prepared: BTreeMap::new(),
        root: None,
        preview_root: None,
        revision: u64::MAX,
        mode: None,
        component: String::new(),
        cache: TextureCache::default(),
        ghost_root: None,
        ghost_key: String::new(),
        preview_yaw: 0.8,
        preview_pitch: 0.45,
        preview_model_pose: Mat4::IDENTITY,
        preview_pose: Transform::default(),
        socket_models: None,
        socket_bounds: None,
        socket_selected: 0,
    });
}
#[allow(clippy::too_many_arguments, reason = "Workshop scene rendering")]
pub fn update(
    mut commands: Commands,
    mut view: ResMut<View>,
    mut editor: ResMut<Editor>,
    catalog: Res<Catalog>,
    nav: Res<Navigation>,
    ui: Res<legacy_ui::State>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut output: ResMut<legacy_ui::Output>,
    mut cameras: Query<(&mut Camera, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let taking_snapshot = editor.snapshot_requested;
    let active =
        nav.page == Page::Workshop || (nav.page == Page::Settings && nav.back == Page::Workshop);
    let eye = Vec3::new(
        view.yaw.sin() * view.pitch.cos(),
        view.pitch.sin(),
        view.yaw.cos() * view.pitch.cos(),
    ) * view.distance;
    let focus = Vec3::Y * editor.document.robot.chassis.height * 0.5;
    view.pose = Transform::from_translation(eye + focus).looking_at(focus, Vec3::Y);
    if let Ok((mut camera, mut pose)) = cameras.get_mut(view.main) {
        camera.is_active = active && editor.mode != Mode::Test;
        *pose = view.pose;
    }
    view.preview_pose = Transform::from_translation(
        Vec3::new(
            view.preview_yaw.sin() * view.preview_pitch.cos(),
            view.preview_pitch.sin(),
            view.preview_yaw.cos() * view.preview_pitch.cos(),
        ) * 2.4,
    )
    .looking_at(Vec3::ZERO, Vec3::Y);
    if let Ok((mut camera, mut pose)) = cameras.get_mut(view.preview) {
        camera.is_active = active && editor.mode == Mode::Components;
        *pose = view.preview_pose;
    }
    for (camera, pose, enabled) in [
        (
            view.main_overlay,
            view.pose,
            (editor.mode == Mode::Components && editor.pending.is_some())
                || matches!(editor.mode, Mode::Wiring | Mode::Overview),
        ),
        (
            view.preview_overlay,
            view.preview_pose,
            editor.mode == Mode::Components,
        ),
    ] {
        if let Ok((mut camera, mut transform)) = cameras.get_mut(camera) {
            camera.is_active = active && enabled;
            *transform = pose;
        }
    }
    if !active {
        view.wiring.hide(&mut commands);
        view.heading.hide(&mut commands);
        return;
    }
    if view.socket_models.is_none() {
        let load = |name: &str,
                    view: &mut View,
                    meshes: &mut Assets<Mesh>,
                    materials: &mut Assets<StandardMaterial>,
                    images: &mut Assets<Image>|
         -> Option<(PreparedModel, (Vec3, Vec3))> {
            let path = catalog.root.join(format!("UI/botlab/{name}.gmf"));
            let model = ra2_formats::ModelImporters::with_ra2()
                .decode(&std::fs::read(&path).ok()?)
                .ok()?
                .model;
            let bounds = model.bounds()?;
            let prepared = render::prepare_model(
                &model,
                &FileTextures::new(path.parent()?),
                &RenderSettings::default(),
                &mut view.cache,
                meshes,
                materials,
                images,
            )
            .ok()?;
            for handle in prepared.material_handles() {
                if let Some(mut material) = materials.get_mut(handle) {
                    // The AP asset is a selection volume. A solid translucent
                    // fill keeps its front surface legible instead of exposing
                    // every back edge through the alpha-cutout outline.
                    material.unlit = true;
                    material.base_color_texture = None;
                    material.base_color = Color::srgba(
                        0.05,
                        1.0,
                        0.02,
                        if name.ends_with("unselected") {
                            0.35
                        } else {
                            0.7
                        },
                    );
                    material.alpha_mode = AlphaMode::Blend;
                    material.cull_mode = Some(bevy::render::render_resource::Face::Back);
                    material.depth_bias = 2.;
                }
            }
            Some((prepared, bounds))
        };
        if let (Some((a, ab)), Some((b, bb))) = (
            load(
                "AP_box_unselected",
                &mut view,
                &mut meshes,
                &mut materials,
                &mut images,
            ),
            load(
                "AP_box_selected",
                &mut view,
                &mut meshes,
                &mut materials,
                &mut images,
            ),
        ) {
            view.socket_models = Some((a, b));
            view.socket_bounds = Some((ab.0.min(bb.0), ab.1.max(bb.1)));
        }
    }
    let pending = editor.pending.clone().unwrap_or_default();
    let ghost_key = editor
        .ghost_model
        .as_ref()
        .filter(|_| !pending.is_empty())
        .map(|m| m.key.clone())
        .unwrap_or_else(|| pending.clone());
    if ghost_key != view.ghost_key || view.ghost_socket != editor.selected_socket {
        if let Some(root) = view.ghost_root.take() {
            commands.entity(root).despawn();
        }
        view.ghost_materials.clear();
        view.ghost_state = None;
        if let Some(asset) = catalog.components.get(&pending) {
            let source = editor
                .ghost_model
                .as_ref()
                .map(|m| &m.model)
                .unwrap_or(&asset.model);
            let mut model = (**source).clone();
            fn tint(m: &mut arena_core::Material) {
                m.alpha = 0.6;
                m.color = [1.0, 1.0, 1.0];
                m.two_sided = true;
                for sub in m.submaterials.values_mut() {
                    tint(sub);
                }
            }
            for m in model.materials.values_mut() {
                tint(m);
            }
            if let Ok(prepared) = render::prepare_model(
                &model,
                &FileTextures::new(asset.source.parent().unwrap()),
                &RenderSettings::default(),
                &mut view.cache,
                &mut meshes,
                &mut materials,
                &mut images,
            ) {
                view.ghost_materials = prepared.material_handles().cloned().collect();
                view.ghost_root = Some(prepared.spawn_on_layer(
                    &mut commands,
                    "Placement preview",
                    Transform::default(),
                    9,
                ));
                if let (Some(root), Some((_, selected)), Some(socket)) = (
                    view.ghost_root,
                    &view.socket_models,
                    placement::selected_socket(source, editor.selected_socket),
                ) {
                    let marker = selected.spawn_on_layer(
                        &mut commands,
                        "Selected placement mount",
                        Transform::from_matrix(marker_pose(socket)),
                        9,
                    );
                    commands.entity(marker).insert(ChildOf(root));
                }
            }
        }
        view.ghost_key = ghost_key;
        view.ghost_socket = editor.selected_socket;
    }
    if let Some(root) = view.ghost_root {
        commands.entity(root).insert((
            Transform::from_matrix(editor.ghost_pose.unwrap_or(Mat4::IDENTITY)),
            Visibility::Visible,
        ));
        if view.ghost_state != Some(editor.ghost_valid) {
            for handle in &view.ghost_materials {
                if let Some(mut material) = materials.get_mut(handle) {
                    material.base_color = if editor.ghost_valid {
                        Color::srgba(1., 1., 1., 0.65)
                    } else {
                        Color::srgba(1., 0.08, 0.06, 0.8)
                    };
                    material.alpha_mode = AlphaMode::Blend;
                    material.unlit = true;
                }
            }
            view.ghost_state = Some(editor.ghost_valid);
        }
    }
    if editor.snapshot_requested {
        editor.snapshot_requested = false;
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::image(
                view.target.clone(),
            ))
            .observe(
                |capture: On<bevy::render::view::screenshot::ScreenshotCaptured>,
                 mut editor: ResMut<Editor>| {
                    let Some(data) = &capture.image.data else {
                        return;
                    };
                    let w = capture.image.width() as usize;
                    let h = capture.image.height() as usize;
                    let bgra = matches!(
                        capture.image.texture_descriptor.format,
                        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
                    );
                    let mut pixels = vec![0; 128 * 128 * 4];
                    for y in 0..128 {
                        for x in 0..128 {
                            let at = ((y * h / 128) * w + x * w / 128) * 4;
                            let to = (y * 128 + x) * 4;
                            if let Some(p) = data.get(at..at + 4) {
                                pixels[to..to + 4].copy_from_slice(p);
                                if bgra {
                                    pixels.swap(to, to + 2);
                                }
                            }
                        }
                    }
                    if let Ok(tga) = arena_core::editor::paint::from_rgba(128, 128, &pixels) {
                        let _ = editor.document.edit("Snapshot", |r| {
                            r.snapshot = Some(tga);
                            Ok(())
                        });
                    }
                },
            );
    }
    for name in [
        "3d placeholder",
        "3d component placeholder",
        "3d wiring placeholder",
        "3d appearance placeholder",
        "3d view placeholder",
        "3d preview placeholder",
    ] {
        output.targets.insert(name.into(), view.target.clone());
    }
    output
        .targets
        .insert("component 3d preview".into(), view.preview_target.clone());
    let end_preview = editor
        .selected
        .filter(|_| editor.mode == Mode::Components && editor.preview_burst_end);
    if view.revision != editor.document.revision
        || view.mode != Some(editor.mode)
        || view.end_preview != end_preview
    {
        if let Some(root) = view.root.take() {
            commands.entity(root).despawn();
        }
        view.action_preview = if editor.mode == Mode::Wiring {
            super::preview::Preview::new(&catalog, &editor.document.robot).ok()
        } else {
            None
        };
        view.models.clear();
        view.assembly = None;
        view.collisions.clear();
        if editor.document.robot.chassis.model.is_some() {
            let mut display_robot = editor.document.robot.clone();
            if let Some(part) = end_preview.and_then(|i| display_robot.components.get_mut(i))
                && part.class == "BurstMotor"
            {
                let start = part
                    .settings
                    .get("start_angle")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.);
                let stroke = part
                    .settings
                    .get("stop_angle")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.);
                part.settings
                    .insert("start_angle".into(), (start + stroke).to_string());
            }
            if let Ok(frozen) = catalog.freeze(&display_robot) {
                let assembly = frozen.assembly;
                view.models = frozen
                    .models
                    .into_iter()
                    .map(|m| (m.key, m.model))
                    .collect();
                view.collisions = view
                    .models
                    .iter()
                    .map(|(key, model)| (key.clone(), arena_runtime::robot::collision_parts(model)))
                    .collect();
                let active: std::collections::BTreeSet<_> = view.models.keys().cloned().collect();
                view.prepared.retain(|key, _| active.contains(key));
                let root = commands
                    .spawn((Transform::default(), Visibility::default()))
                    .id();
                if matches!(editor.mode, Mode::Components | Mode::Wiring) {
                    spawn_baseplate(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        root,
                        &view.models["@chassis"],
                        view.baseplate_texture.clone(),
                    );
                    let mesh = &view.models["@chassis"].objects[0];
                    let n = editor.document.robot.chassis.bottom.len();
                    let mut lines = vec![];
                    for i in 0..n {
                        let j = (i + 1) % n;
                        for [a, b] in [[i, j], [n + i, n + j], [i, n + i]] {
                            lines.extend([mesh.vertices[a], mesh.vertices[b]]);
                        }
                    }
                    spawn_lines(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        root,
                        lines,
                        [0.7, 0.7, 0.7],
                    );
                }
                for part in assembly.parts.iter().filter(|p| p.visible) {
                    if part.component == 0 && matches!(editor.mode, Mode::Components | Mode::Wiring)
                    {
                        continue;
                    }

                    let model = if part.component == 0 {
                        let mut cache = TextureCache::default();
                        let bytes = editor
                            .document
                            .robot
                            .chassis
                            .texture
                            .as_deref()
                            .unwrap_or_default();
                        render::prepare_model(
                            &view.models["@chassis"],
                            &PaintTexture(bytes),
                            &RenderSettings {
                                roughness: (1.0
                                    - editor.document.robot.chassis.material_parameters[4] / 120.0)
                                    .clamp(0.08, 0.85),
                                ..Default::default()
                            },
                            &mut cache,
                            &mut meshes,
                            &mut materials,
                            &mut images,
                        )
                        .ok()
                    } else {
                        if !view.prepared.contains_key(&part.model)
                            && let Some(asset) = catalog.components.get(&normalize(
                                &editor.document.robot.components[part.component].definition,
                            ))
                        {
                            let source = FileTextures::new(asset.source.parent().unwrap());
                            let posed_model = view.models[&part.model].clone();
                            if let Ok(prepared) = render::prepare_model(
                                &posed_model,
                                &source,
                                &RenderSettings::default(),
                                &mut view.cache,
                                &mut meshes,
                                &mut materials,
                                &mut images,
                            ) {
                                view.prepared.insert(part.model.clone(), prepared);
                            }
                        }
                        view.prepared.get(&part.model).cloned()
                    };
                    if let Some(model) = model {
                        let (entity, objects) = model.spawn_tracked(
                            &mut commands,
                            &format!("Editor part {}", part.component),
                            Transform::from_matrix(part.transform),
                            7,
                        );
                        commands.entity(entity).insert(ChildOf(root));
                        let original_objects = view.models[&part.model]
                            .objects
                            .iter()
                            .map(|o| part.transform * o.transform)
                            .collect::<Vec<_>>();
                        if let Some(preview) = &mut view.action_preview {
                            for (index, object_entity) in objects {
                                let body = preview.rig.objects[part.component][index];
                                // Filled from the rendered frozen model; one transform per
                                // material group, no mesh rebuilds while keys are held.
                                preview.objects.push((
                                    object_entity,
                                    body,
                                    part.transform.inverse(),
                                    original_objects[index],
                                ));
                            }
                        }
                    }
                }
                editor.document.robot.weight_class =
                    arena_core::editor::weight_class(assembly.mass);
                view.root = Some(root);
                view.assembly = Some(assembly);
            }
        }
        view.revision = editor.document.revision;
        view.mode = Some(editor.mode);
        view.end_preview = end_preview;
    }
    if let Some(preview) = &mut view.action_preview {
        preview.update(
            &mut commands,
            &editor.document.robot,
            &keys,
            nav.page == Page::Workshop && editor.dialog.is_none() && editor.capture_key.is_none(),
            time.delta_secs(),
        );
    }
    let View {
        heading, assembly, ..
    } = &mut *view;
    heading.update(
        &mut commands,
        &editor,
        assembly.as_ref(),
        &ui,
        time.elapsed_secs_f64(),
        &catalog.root,
        &mut meshes,
        &mut materials,
        &mut images,
    );
    if taking_snapshot {
        view.heading.hide(&mut commands);
    }
    // Separate from the scene geometry: these pointers never affect picking or physics.
    // Layer 11 also keeps them out of the garage camera, which shares ghost layer 9.
    let View {
        wiring,
        assembly,
        models,
        pose,
        root,
        ..
    } = &mut *view;
    wiring.update(
        &mut commands,
        &editor,
        assembly.as_ref(),
        models,
        *pose,
        *root,
        &mut meshes,
        &mut materials,
    );
    if view.component != editor.component || view.socket_selected != editor.selected_socket {
        if let Some(root) = view.preview_root.take() {
            commands.entity(root).despawn();
        }
        view.preview_model = None;
        if let Some(asset) = catalog.components.get(&editor.component)
            && let Ok(template) = catalog.preview(&editor.component)
        {
            let source = FileTextures::new(asset.source.parent().unwrap());
            if let Ok(model) = render::prepare_model(
                &template.model,
                &source,
                &RenderSettings::default(),
                &mut view.cache,
                &mut meshes,
                &mut materials,
                &mut images,
            ) && let Some((min, max)) = template.model.bounds()
            {
                let scale = 1.5 / (max - min).max_element().max(0.01);
                let pose = Transform::from_translation(-(min + max) * 0.5 * scale)
                    .with_scale(Vec3::splat(scale));
                let root = model.spawn_on_layer(&mut commands, asset.name(), pose, 8);
                view.preview_model_pose = pose.to_matrix();
                if let Some((normal, selected)) = &view.socket_models {
                    for socket in &template.model.attachments {
                        let is_selected = socket
                            .properties
                            .get("id")
                            .and_then(|s| s.parse::<u32>().ok())
                            == Some(editor.selected_socket);
                        let marker = if is_selected { selected } else { normal }.spawn_on_layer(
                            &mut commands,
                            "Attachment point",
                            Transform::from_matrix(marker_pose(socket)),
                            10,
                        );
                        commands.entity(marker).insert(ChildOf(root));
                    }
                }
                view.preview_root = Some(root);
                view.preview_model = Some(template.model);
            }
        }
        view.component = editor.component.clone();
        view.socket_selected = editor.selected_socket;
    }
    let mount_stamp = (
        editor.document.revision,
        editor.pending.clone().unwrap_or_default(),
        editor.selected_socket,
    );
    if view.mount_stamp != mount_stamp {
        if let Some(root) = view.mount_root.take() {
            commands.entity(root).despawn();
        }
        view.mounts.clear();
        if editor.pending.is_some()
            && let (Some(assembly), Some((normal, selected))) =
                (&view.assembly, &view.socket_models)
        {
            let root = commands
                .spawn((Transform::default(), Visibility::default()))
                .id();
            let mut mounts = Vec::new();
            for part in assembly
                .parts
                .iter()
                .filter(|p| p.component > 0 && p.visible)
            {
                if let Some(model) = view.models.get(&part.model) {
                    for socket in &model.attachments {
                        let Some(id) = placement::socket_id(socket) else {
                            continue;
                        };
                        let own_type = editor
                            .pending
                            .as_ref()
                            .and_then(|key| catalog.components.get(key))
                            .and_then(|a| {
                                placement::selected_socket(&a.model, editor.selected_socket)
                            })
                            .and_then(|a| a.properties.get("attach"))
                            .map_or("", String::as_str);
                        if !placement::available_target(&editor, own_type, part.component, socket) {
                            continue;
                        }
                        let pose = part.transform * marker_pose(socket);
                        let normal_root = normal.spawn_on_layer(
                            &mut commands,
                            "Target attachment",
                            Transform::from_matrix(pose),
                            9,
                        );
                        let selected_root = selected.spawn_on_layer(
                            &mut commands,
                            "Selected target attachment",
                            Transform::from_matrix(pose),
                            9,
                        );
                        commands.entity(normal_root).insert(ChildOf(root));
                        commands.entity(selected_root).insert(ChildOf(root));
                        let frame = part.transform * socket.transform;
                        mounts.push(Mount {
                            pick: Pick {
                                attachment: Some(id),
                                component: part.component,
                                position: frame.transform_point3(Vec3::ZERO),
                                normal: -frame.transform_vector3(Vec3::Z).normalize(),
                                face: 0,
                                uv: Vec2::ZERO,
                            },
                            pose,
                            normal: normal_root,
                            selected: selected_root,
                        });
                    }
                }
            }
            view.mount_root = Some(root);
            view.mounts = mounts;
        }
        view.mount_stamp = mount_stamp;
    }
    for marker in &view.mounts {
        let selected = editor.ghost_pick.as_ref().is_some_and(|p| {
            p.component == marker.pick.component && p.attachment == marker.pick.attachment
        });
        commands.entity(marker.normal).insert(if selected {
            Visibility::Hidden
        } else {
            Visibility::Visible
        });
        commands.entity(marker.selected).insert(if selected {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }
}
fn spawn_baseplate(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    parent: Entity,
    model: &Model,
    texture: Option<Handle<Image>>,
) {
    let mut positions = Vec::new();
    let mut uv = Vec::new();
    for object in model.objects.iter().filter(|o| o.visible) {
        let vertices: Vec<_> = object.positions().collect();
        let min_y = vertices.iter().map(|v| v.y).reduce(f32::min).unwrap_or(0.);
        for face in &object.faces {
            let p = face.vertices.map(|i| vertices[i]);
            if p.iter().all(|p| (p.y - min_y).abs() < 1e-4) {
                for mut p in p {
                    uv.push([p.x * 8., p.z * 8.]);
                    p.y += 0.0005;
                    positions.push(p.to_array());
                }
            }
        }
    }
    let count = positions.len();
    let mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    commands.spawn((
        Name::new("Original green baseplate grid"),
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.5, 0.5),
            base_color_texture: texture,
            double_sided: true,
            cull_mode: None,
            unlit: true,
            ..default()
        })),
        Transform::default(),
        RenderLayers::layer(7),
        ChildOf(parent),
    ));
}

fn spawn_lines(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    parent: Entity,
    points: Vec<Vec3>,
    color: [f32; 3],
) {
    let count = points.len();
    let mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::LineList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        points.into_iter().map(|p| p.to_array()).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count]);
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(color[0], color[1], color[2]),
            unlit: true,
            ..Default::default()
        })),
        Transform::default(),
        RenderLayers::layer(7),
        ChildOf(parent),
    ));
}
impl View {
    pub(crate) fn preview_entities(&self) -> Vec<Entity> {
        self.action_preview
            .as_ref()
            .map_or_else(Vec::new, |p| p.objects.iter().map(|o| o.0).collect())
    }
    pub fn preview_socket(
        &self,
        point: Vec2,
        region: Rect,
        catalog: &Catalog,
        key: &str,
    ) -> Option<u32> {
        let asset = catalog.components.get(key)?;
        let p = (point - region.min) / region.size();
        let t = (std::f32::consts::FRAC_PI_4 * 0.5).tan();
        let direction = self.preview_pose.rotation
            * Vec3::new((p.x * 2.0 - 1.0) * t, (1.0 - p.y * 2.0) * t, -1.0).normalize();
        let origin = self.preview_pose.translation;
        pick_socket(
            self.preview_model.as_deref().unwrap_or(&asset.model),
            self.preview_model_pose,
            self.socket_bounds?,
            origin,
            direction,
        )
    }

    pub fn ray(&self, point: Vec2, region: Rect) -> (Vec3, Vec3) {
        let p = (point - region.min) / region.size();
        let tangent = (std::f32::consts::FRAC_PI_4 * 0.5).tan();
        let direction = Vec3::new(
            (p.x * 2.0 - 1.0) * tangent,
            (1.0 - p.y * 2.0) * tangent,
            -1.0,
        )
        .normalize();
        (self.pose.translation, self.pose.rotation * direction)
    }
    pub fn floor_pick(&self, point: Vec2, region: Rect) -> Pick {
        let (origin, direction) = self.ray(point, region);
        let t = if direction.y.abs() > 1e-5 {
            -origin.y / direction.y
        } else {
            self.distance
        };
        let mut position = origin + direction * t.clamp(0.1, self.distance * 3.);
        position.y = 0.;
        Pick {
            attachment: None,
            component: 0,
            position,
            normal: Vec3::Y,
            face: usize::MAX,
            uv: Vec2::ZERO,
        }
    }
    pub fn pick(&self, point: Vec2, region: Rect, skip_chassis: bool) -> Option<Pick> {
        let (origin, direction) = self.ray(point, region);
        if let Some(bounds) = self.socket_bounds
            && let Some(marker) = self
                .mounts
                .iter()
                .filter_map(|m| ray_box(origin, direction, m.pose, bounds).map(|t| (t, m)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            return Some(marker.1.pick.clone());
        }
        self.pick_geometry(point, region, skip_chassis, false)
    }
    /// Base-mounted parts follow the baseplate under the pointer, including
    /// between other parts. Only explicit, filtered connector markers snap.
    pub fn placement_pick(&self, point: Vec2, region: Rect, base: bool) -> Option<Pick> {
        let (origin, direction) = self.ray(point, region);
        if let Some(bounds) = self.socket_bounds
            && let Some((_, marker)) = self
                .mounts
                .iter()
                .filter_map(|m| ray_box(origin, direction, m.pose, bounds).map(|t| (t, m)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            return Some(marker.pick.clone());
        }
        self.pick_geometry(point, region, false, base)
    }
    fn pick_geometry(
        &self,
        point: Vec2,
        region: Rect,
        skip_chassis: bool,
        only_chassis: bool,
    ) -> Option<Pick> {
        let (origin, direction) = self.ray(point, region);
        let assembly = self.assembly.as_ref()?;
        let mut nearest = f32::INFINITY;
        let mut hit = None;
        for part in assembly.parts.iter().filter(|p| {
            p.visible && (!skip_chassis || p.component != 0) && (!only_chassis || p.component == 0)
        }) {
            let model = self.models.get(&part.model)?;
            for object in &model.objects {
                if !object.visible {
                    continue;
                }
                let pose = part.transform * object.transform;
                for (face_index, face) in object.faces.iter().enumerate() {
                    let [a, b, c] = face
                        .vertices
                        .map(|i| pose.transform_point3(object.vertices[i]));
                    if part.component == 0
                        && matches!(self.mode, Some(Mode::Components | Mode::Wiring))
                    {
                        let floor = model.bounds().map_or(0., |(min, _)| min.y);
                        if [a, b, c].iter().any(|p| (p.y - floor).abs() > 1e-4) {
                            continue;
                        }
                    }
                    let edge1 = b - a;
                    let edge2 = c - a;
                    let p = direction.cross(edge2);
                    let det = edge1.dot(p);
                    if det.abs() < 1e-7 {
                        continue;
                    }
                    let inverse = det.recip();
                    let t = origin - a;
                    let u = t.dot(p) * inverse;
                    let q = t.cross(edge1);
                    let v = direction.dot(q) * inverse;
                    let distance = edge2.dot(q) * inverse;
                    if u < 0.0 || v < 0.0 || u + v > 1.0 || distance < 0.0 || distance >= nearest {
                        continue;
                    }
                    nearest = distance;
                    hit = Some(Pick {
                        attachment: None,
                        component: part.component,
                        position: origin + direction * distance,
                        normal: edge1.cross(edge2).normalize_or_zero(),
                        face: face_index,
                        uv: Vec2::from_array(face.uv[0]) * (1.0 - u - v)
                            + Vec2::from_array(face.uv[1]) * u
                            + Vec2::from_array(face.uv[2]) * v,
                    });
                }
            }
        }
        hit
    }
}

/// AP_box is authored along +Y; a socket's outward normal is local -Z.
/// Keep this correction shared by rendering and mouse picking.
fn marker_pose(socket: &arena_core::Attachment) -> Mat4 {
    socket.transform * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

fn pick_socket(
    model: &Model,
    model_pose: Mat4,
    bounds: (Vec3, Vec3),
    origin: Vec3,
    direction: Vec3,
) -> Option<u32> {
    model
        .attachments
        .iter()
        .filter_map(|socket| {
            let id = placement::socket_id(socket)?;
            let distance = ray_box(origin, direction, model_pose * marker_pose(socket), bounds)?;
            Some((distance, id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// Pick the whole hollow tube, including its opening. The unnormalized local ray
/// preserves world-space distance under the component preview's scaling.
fn ray_box(origin: Vec3, direction: Vec3, pose: Mat4, (min, max): (Vec3, Vec3)) -> Option<f32> {
    let inverse = pose.inverse();
    let origin = inverse.transform_point3(origin);
    let direction = inverse.transform_vector3(direction);
    let mut near = 0.0_f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        if direction[axis].abs() < 1e-8 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
        } else {
            let a = (min[axis] - origin[axis]) / direction[axis];
            let b = (max[axis] - origin[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
    }
    (near.is_finite() && far >= near).then_some(near)
}

#[cfg(test)]
mod socket_tests {
    use super::*;

    #[test]
    fn marker_tracks_outward_socket_axis_and_can_be_picked_at_its_tip() {
        let bounds = (
            Vec3::new(-0.033, -0.01465, -0.033),
            Vec3::new(0.033, 0.08887, 0.033),
        );
        let socket = |id: u32, pose| arena_core::Attachment {
            name: String::new(),
            transform: pose,
            properties: [("id".into(), id.to_string())].into(),
        };
        let model = Model {
            attachments: vec![
                socket(
                    2,
                    Mat4::from_rotation_translation(
                        Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                        Vec3::X,
                    ),
                ),
                socket(1, Mat4::IDENTITY),
            ],
            ..Default::default()
        };
        for scale in [0.5, 1.0, 6.0] {
            let pose = Mat4::from_scale_rotation_translation(
                Vec3::splat(scale),
                Quat::from_rotation_y(0.8),
                Vec3::new(-0.2, 0.1, 0.4),
            );
            for a in &model.attachments {
                let frame = pose * marker_pose(a);
                let direction = frame.transform_vector3(Vec3::Y).normalize();
                let expected = -(pose * a.transform).transform_vector3(Vec3::Z).normalize();
                assert!(direction.dot(expected) > 0.9999);
                let tip = frame.transform_point3(Vec3::Y * 0.08);
                let origin = tip + frame.transform_vector3(Vec3::X).normalize() * 3.0;
                assert_eq!(
                    pick_socket(&model, pose, bounds, origin, (tip - origin).normalize()),
                    placement::socket_id(a)
                );
            }
        }
        assert!(ray_box(Vec3::new(10.0, 0.0, 1.0), -Vec3::Z, Mat4::IDENTITY, bounds).is_none());
        assert!(ray_box(Vec3::Z, Vec3::Z, Mat4::IDENTITY, bounds).is_none());
    }

    #[test]
    fn overlapping_socket_markers_select_the_nearest_not_first_id() {
        let bounds = (Vec3::splat(-0.05), Vec3::splat(0.05));
        let model = Model {
            attachments: [1, 2]
                .into_iter()
                .map(|id| arena_core::Attachment {
                    name: String::new(),
                    transform: Mat4::from_translation(Vec3::Z * id as f32),
                    properties: [("id".into(), id.to_string())].into(),
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            pick_socket(&model, Mat4::IDENTITY, bounds, Vec3::Z * 4.0, -Vec3::Z),
            Some(2)
        );
    }
}
