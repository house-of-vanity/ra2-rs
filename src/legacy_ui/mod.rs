//! Runtime for original GIB widgets, original images and original GIFF glyphs.
pub mod assets;
pub mod canvas;
pub(crate) mod floating;
mod input;
pub mod layout;
pub(crate) mod widgets;
use crate::frontend::{MenuCamera, Navigation};
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use ra2_formats::gib::Widget;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Default)]
pub struct Cell {
    pub key: String,
    pub label: String,
    pub image: String,
    pub selected: bool,
    pub template: Option<Arc<Widget>>,
}
#[derive(Clone)]
pub struct Lines {
    pub region: String,
    pub points: Vec<Vec2>,
    pub color: [f32; 4],
    pub width: f32,
    pub dots: bool,
    pub dot_size: f32,
    pub dot_image: Option<String>,
}
#[derive(Clone)]
pub struct FloatingText {
    pub region: String,
    pub point: Vec2,
    /// Logical reference pixels; separate opposite damage directions at one contact.
    pub offset: Vec2,
    pub text: String,
    pub color: [f32; 4],
    pub diagnostics: bool,
}
#[derive(Resource, Default)]
pub struct State {
    pub root: Option<Arc<Widget>>,
    pub slots: BTreeMap<String, Arc<Widget>>,
    pub dialogs: Vec<Arc<Widget>>,
    pub values: BTreeMap<String, String>,
    pub checked: BTreeMap<String, bool>,
    pub disabled: BTreeSet<String>,
    pub cells: BTreeMap<String, Vec<Cell>>,
    pub viewports: BTreeSet<String>,
    pub lines: Vec<Lines>,
    pub floating_text: Vec<FloatingText>,
    pub scroll: BTreeMap<String, usize>,
    pub hover: String,
    pub pressed: String,
    pub focused: String,
    pub revision: u64,
    pub attachments: Vec<(String, Arc<Widget>)>,
    pub interactive: BTreeSet<String>,
    pub images: BTreeMap<String, String>,
    pub colors: BTreeMap<String, [f32; 4]>,
    pub choices: BTreeMap<String, Vec<String>>,
    pub popup: Option<String>,
    pub visibility: BTreeMap<String, bool>,
    pub control_drag: Option<(String, Vec2)>,
}
#[derive(Clone, Debug)]
pub enum UiEvent {
    Move {
        name: String,
        point: Vec2,
        delta: Vec2,
    },
    Wheel {
        name: String,
        delta: f32,
    },
    Activate {
        name: String,
        owner: String,
        point: Vec2,
    },
    Value {
        name: String,
        value: String,
    },
    Press {
        name: String,
        point: Vec2,
        button: MouseButton,
    },
    Drag {
        name: String,
        point: Vec2,
        delta: Vec2,
        button: MouseButton,
    },
    Release {
        name: String,
        point: Vec2,
        button: MouseButton,
    },
}
#[derive(Resource, Default)]
pub struct Events(pub Vec<UiEvent>);
#[derive(Resource)]
pub struct Output {
    pub image: Handle<Image>,
    pub root: Entity,
    pub viewport_root: Entity,
    pub hits: Vec<layout::Hit>,
    pub scroll_limits: BTreeMap<String, usize>,
    pub regions: BTreeMap<String, Rect>,
    pub scale: f32,
    pub origin: Vec2,
    pub size: UVec2,
    pub rendered: u64,
    pub visible: bool,
    pub targets: BTreeMap<String, Handle<Image>>,
}
#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Input;
#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Paint;
pub struct LegacyUiPlugin;
impl Plugin for LegacyUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<State>()
            .init_resource::<Events>()
            .add_systems(
                Startup,
                (assets::load, setup)
                    .chain()
                    .after(crate::frontend::FrontendSetup),
            )
            .add_systems(Update, input::input.in_set(Input))
            .add_systems(Update, paint.in_set(Paint))
            .add_systems(Startup, floating::setup.after(setup))
            .add_systems(
                Update,
                floating::paint
                    .after(crate::gameplay::numbers::project)
                    .after(Paint),
            );
    }
}
fn setup(
    mut commands: Commands,
    cameras: Query<Entity, With<MenuCamera>>,
    mut images: ResMut<Assets<Image>>,
) {
    let image = images.add(Image::new(
        Extent3d {
            width: 640,
            height: 480,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; 640 * 480 * 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ));
    let camera = cameras
        .single()
        .expect("Menu camera ready before legacy UI");
    let root = commands
        .spawn((
            UiTargetCamera(camera),
            GlobalZIndex(17),
            ImageNode::new(image.clone()),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                display: Display::None,
                ..default()
            },
            bevy::picking::Pickable::IGNORE,
        ))
        .id();
    let viewport_root = commands
        .spawn((
            UiTargetCamera(camera),
            GlobalZIndex(16),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                display: Display::None,
                ..default()
            },
            bevy::picking::Pickable::IGNORE,
        ))
        .id();
    commands.insert_resource(Output {
        image,
        root,
        viewport_root,
        hits: vec![],
        scroll_limits: BTreeMap::new(),
        regions: BTreeMap::new(),
        scale: 1.0,
        origin: Vec2::ZERO,
        size: UVec2::ZERO,
        rendered: u64::MAX,
        visible: false,
        targets: BTreeMap::new(),
    });
}
#[allow(clippy::too_many_arguments, reason = "Legacy UI rendering resources")]
fn paint(
    mut commands: Commands,
    state: Res<State>,
    assets: Res<assets::LegacyAssets>,
    nav: Res<Navigation>,
    windows: Query<&Window>,
    mut output: ResMut<Output>,
    mut images: ResMut<Assets<Image>>,
    mut nodes: Query<&mut Node>,
) {
    let visible = nav.legacy_visible();
    if visible != output.visible {
        for entity in [output.root, output.viewport_root] {
            if let Ok(mut node) = nodes.get_mut(entity) {
                node.display = if visible {
                    Display::Flex
                } else {
                    Display::None
                };
            }
        }
        output.visible = visible;
    }
    if !visible {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    // Keep the original 4:3 composition and pixel metrics at the reference 800x600 mode.
    // Widescreen windows receive side margins instead of stretching the workshop controls.
    let size = UVec2::new(800, 600);
    let scale = (window.width() / size.x as f32)
        .min(window.height() / size.y as f32)
        .max(0.1);
    let origin = (Vec2::new(window.width(), window.height()) - size.as_vec2() * scale) * 0.5;
    if output.rendered == state.revision
        && output.size == size
        && output.scale == scale
        && output.origin == origin
    {
        return;
    }
    for entity in [output.root, output.viewport_root] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.left = px(origin.x);
            node.top = px(origin.y);
            node.width = px(size.x as f32 * scale);
            node.height = px(size.y as f32 * scale);
        }
    }
    let mut frame = layout::compose(&assets, &state, size.x as usize, size.y as usize);
    for lines in &state.lines {
        if let Some(region) = frame.regions.get(&lines.region) {
            for pair in lines.points.windows(2).filter(|_| lines.width > 0.) {
                frame.pixels.line(
                    region.min + pair[0] * region.size(),
                    region.min + pair[1] * region.size(),
                    lines.width,
                    lines.color,
                    *region,
                );
            }
            if lines.dots {
                for p in &lines.points {
                    let rect = Rect::from_center_size(
                        region.min + *p * region.size(),
                        Vec2::splat(lines.dot_size),
                    );
                    if let Some(image) = lines.dot_image.as_ref().and_then(|key| assets.image(key))
                    {
                        frame.pixels.image(image, rect, lines.color, true, *region);
                    } else {
                        frame.pixels.fill(rect, lines.color, *region);
                    }
                }
            }
        }
    }
    if let Some(mut image) = images.get_mut(&output.image) {
        image.resize(Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
        image.data = Some(frame.pixels.rgba);
        image.sampler = ImageSampler::nearest();
    }
    commands
        .entity(output.viewport_root)
        .despawn_related::<Children>();
    for name in &state.viewports {
        if let (Some(rect), Some(target)) = (frame.regions.get(name), output.targets.get(name)) {
            commands.spawn((
                ImageNode::new(target.clone()).with_mode(bevy::ui::widget::NodeImageMode::Stretch),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(rect.min.x * scale),
                    top: px(rect.min.y * scale),
                    width: px(rect.width() * scale),
                    height: px(rect.height() * scale),
                    ..default()
                },
                ChildOf(output.viewport_root),
                bevy::picking::Pickable::IGNORE,
            ));
        }
    }
    if std::env::var_os("RA2_UI_PROBE").is_some() {
        let mut text = format!("origin\t{}\t{}\tscale\t{}\n", origin.x, origin.y, scale);
        for hit in &frame.hits {
            let r = hit.rect.intersect(hit.clip);
            if !r.is_empty() {
                text.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                    hit.name,
                    hit.owner,
                    r.min.x,
                    r.min.y,
                    r.width(),
                    r.height()
                ));
            }
        }
        let _ = std::fs::write("logs/editor-hit-regions.tsv", text);
    }
    output.hits = frame.hits;
    output.scroll_limits = frame.scroll_limits;
    output.regions = frame.regions;
    output.scale = scale;
    output.origin = origin;
    output.size = size;
    output.rendered = state.revision;
}
