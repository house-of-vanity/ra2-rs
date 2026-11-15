//! Pooled GPU text, rasterized at actual font size/DPI, above the native bitmap UI.
use super::*;
#[derive(Component)]
pub(super) struct FloatingRoot;
#[derive(Component)]
pub(super) struct ClipBox;
#[derive(Component)]
pub(crate) struct DamageLabel;
#[derive(Resource)]
pub(super) struct Pool {
    root: Entity,
    labels: Vec<(Entity, Entity)>,
}
pub(super) fn setup(mut commands: Commands, cameras: Query<Entity, With<MenuCamera>>) {
    let root = commands
        .spawn((
            FloatingRoot,
            UiTargetCamera(cameras.single().unwrap()),
            GlobalZIndex(18),
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
    commands.insert_resource(Pool {
        root,
        labels: vec![],
    });
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn paint(
    mut commands: Commands,
    mut pool: ResMut<Pool>,
    state: Res<State>,
    output: Res<Output>,
    nav: Res<Navigation>,
    windows: Query<&Window>,
    mut roots: Query<&mut Node, (With<FloatingRoot>, Without<ClipBox>, Without<DamageLabel>)>,
    mut boxes: Query<&mut Node, (With<ClipBox>, Without<FloatingRoot>, Without<DamageLabel>)>,
    mut texts: Query<
        (
            &mut Node,
            &mut Text,
            &mut TextFont,
            &mut TextColor,
            &mut TextShadow,
            &mut TextLayout,
        ),
        (With<DamageLabel>, Without<ClipBox>, Without<FloatingRoot>),
    >,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let count = if nav.legacy_visible() {
        state.floating_text.len().min(128)
    } else {
        0
    };
    if let Ok(mut node) = roots.get_mut(pool.root) {
        node.display = if count == 0 {
            Display::None
        } else {
            Display::Flex
        };
    }
    for i in 0..count {
        let label = &state.floating_text[i];
        let bounds = if label.region == "@screen" {
            Rect::from_corners(Vec2::ZERO, Vec2::new(window.width(), window.height()))
        } else if let Some(rect) = output.regions.get(&label.region) {
            Rect::from_corners(
                output.origin + rect.min * output.scale,
                output.origin + rect.max * output.scale,
            )
        } else {
            continue;
        };
        let size = if label.diagnostics { 14. } else { 22. } * output.scale;
        let half_width = if label.diagnostics {
            0.
        } else {
            70. * output.scale
        };
        let layout = TextLayout::justify(if label.diagnostics {
            Justify::Left
        } else {
            Justify::Center
        });
        let node = Node {
            position_type: PositionType::Absolute,
            left: px(label.point.x * bounds.width() + label.offset.x * output.scale - half_width),
            top: px(label.point.y * bounds.height() + label.offset.y * output.scale - size * 0.5),
            width: px(if label.diagnostics {
                bounds.width() * 0.96
            } else {
                140. * output.scale
            }),
            ..default()
        };
        let clip_node = Node {
            position_type: PositionType::Absolute,
            left: px(bounds.min.x),
            top: px(bounds.min.y),
            width: px(bounds.width()),
            height: px(bounds.height()),
            overflow: Overflow::clip(),
            ..default()
        };
        let color = Color::srgba(
            label.color[0],
            label.color[1],
            label.color[2],
            label.color[3],
        );
        let shadow = TextShadow {
            offset: Vec2::splat(output.scale.max(1.)),
            color: Color::srgba(0., 0., 0., label.color[3] * 0.9),
        };
        if let Some(&(container, text)) = pool.labels.get(i) {
            if let Ok(mut current) = boxes.get_mut(container) {
                *current = clip_node;
            }
            if let Ok((
                mut current,
                mut text,
                mut font,
                mut tint,
                mut current_shadow,
                mut current_layout,
            )) = texts.get_mut(text)
            {
                *current = node;
                if text.0 != label.text {
                    text.0 = label.text.clone();
                }
                if font.font_size != FontSize::Px(size) {
                    font.font_size = FontSize::Px(size);
                }
                tint.0 = color;
                *current_shadow = shadow;
                *current_layout = layout;
            }
        } else {
            let container = commands
                .spawn((
                    ClipBox,
                    ChildOf(pool.root),
                    clip_node,
                    bevy::picking::Pickable::IGNORE,
                ))
                .id();
            let text = commands
                .spawn((
                    DamageLabel,
                    ChildOf(container),
                    node,
                    Text::new(&label.text),
                    TextFont {
                        font_size: FontSize::Px(size),
                        ..default()
                    },
                    layout,
                    TextColor(color),
                    shadow,
                    bevy::picking::Pickable::IGNORE,
                ))
                .id();
            pool.labels.push((container, text));
        }
    }
    for &(container, _) in pool.labels.iter().skip(count) {
        if let Ok(mut node) = boxes.get_mut(container) {
            node.display = Display::None;
        }
    }
}
