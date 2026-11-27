use super::*;
use bevy::{
    input_focus::{FocusCause, InputFocus},
    picking::{Pickable, hover::Hovered},
    text::{EditableText, TextCursorStyle},
    ui_widgets::{
        Activate, ActivateOnPress, Button, Slider, SliderPrecision, SliderRange, SliderStep,
        SliderThumb, SliderValue, TextInput, TrackClick, ValueChange,
    },
};
#[derive(Component)]
pub(super) struct RobotSearch;

const PAGE: usize = 8;
const LEFT: f32 = 270.0;
const RIGHT: f32 = 340.0;
#[derive(Component, Clone, Copy)]
pub(super) enum Action {
    Slot(usize),
    Page(i32),
    Clear,
    Wire,
    Shading,
    Boxes,
    Impact,
    Pause,
    Reset,
    Tab(usize),
    Particles,
    Wear,
    ClearEffects,
    ClearDebris,
    Quit,
}
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(super) enum Control {
    Power,
    Radius,
    Depth,
    Limit,
    Density,
    Sparks,
    Chips,
    ChipSize,
    DebrisLimit,
    Speed,
    Lifetime,
    WearStrength,
    WearRadius,
}
#[derive(Component)]
pub(super) enum Label {
    Slot(usize),
    Page,
    Robot,
    Toggle(Action),
    Value(Control),
    Fps,
    Rigid,
    Damage,
    EffectsHit,
    Surface,
    Fx,
    Debris,
    Detached,
    Mesh,
    Strikes,
    Components,
    Status,
}
#[derive(Component)]
pub(super) struct Section(usize);
#[derive(Component)]
pub(super) struct Thumb(Entity);
#[derive(Component)]
pub(super) struct ScrollPanel(bool);

impl Control {
    fn value(self, lab: &Lab) -> f32 {
        match self {
            Self::Power => lab.power,
            Self::Radius => lab.radius,
            Self::Depth => lab.depth,
            Self::Limit => lab.max_depth,
            Self::Density => lab.density_preview,
            Self::Sparks => lab.effect_settings.sparks,
            Self::Chips => lab.effect_settings.chips,
            Self::ChipSize => lab.effect_settings.chip_scale,
            Self::DebrisLimit => lab.effect_settings.debris_limit as f32,
            Self::Speed => lab.effect_settings.speed,
            Self::Lifetime => lab.effect_settings.lifetime,
            Self::WearStrength => lab.effect_settings.wear_strength,
            Self::WearRadius => lab.effect_settings.wear_radius,
        }
    }
    fn spec(self) -> (&'static str, f32, f32, f32, i32) {
        match self {
            Self::Power => ("Hammer power", 0.25, 5., 0.05, 2),
            Self::Radius => ("Dent radius", 0.2, 1.5, 0.01, 2),
            Self::Depth => ("Dent per hit", 0.01, 0.20, 0.001, 3),
            Self::Limit => ("Maximum dent", 0.05, 0.8, 0.01, 2),
            Self::Density => ("Mesh density", 0., 4., 1., 0),
            Self::Sparks => ("Sparks per hit", 0., 100., 1., 0),
            Self::Chips => ("Paint / metal chips", 0., 80., 1., 0),
            Self::ChipSize => ("Chip / fragment size", 0.5, 3.0, 0.05, 2),
            Self::DebrisLimit => (
                "Ground debris limit",
                0.0,
                effects::MAX_DEBRIS as f32,
                256.0,
                0,
            ),
            Self::Speed => ("Particle speed", 0.25, 3., 0.05, 2),
            Self::Lifetime => ("Spark lifetime", 0.2, 2., 0.05, 2),
            Self::WearStrength => ("Paint loss visibility", 0., 1., 0.01, 2),
            Self::WearRadius => ("Chips / scratch radius", 0.02, 0.2, 0.005, 3),
        }
    }
    fn caption(self, value: f32) -> String {
        match self {
            Self::Power | Self::Speed | Self::ChipSize => format!("{value:.2}x"),
            Self::Density => format!("{}x", MESH_DENSITIES[(value.round() as usize).min(4)]),
            Self::Radius | Self::WearRadius => format!("{:.1} cm", value * 100.),
            Self::Depth | Self::Limit => format!("{:.0} mm", value * 1000.),
            Self::Sparks | Self::Chips | Self::DebrisLimit => format!("{value:.0}"),
            Self::Lifetime => format!("{value:.2} s"),
            Self::WearStrength => format!("{:.0}%", value * 100.),
        }
    }
}
fn text(commands: &mut Commands, parent: Entity, value: &str, size: f32) -> Entity {
    commands
        .spawn((
            Text::new(value),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(Color::srgb(0.86, 0.90, 0.94)),
            Node {
                flex_shrink: 0.,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}
fn row(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Node {
                width: percent(100),
                column_gap: px(8),
                align_items: AlignItems::Center,
                flex_shrink: 0.,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}
fn column(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                flex_shrink: 0.,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}
fn button(commands: &mut Commands, parent: Entity, caption: &str, action: Action) -> Entity {
    let entity = commands
        .spawn((
            Button,
            ActivateOnPress,
            Hovered::default(),
            action,
            Node {
                flex_grow: 1.,
                min_height: px(30),
                padding: UiRect::all(px(6)),
                border: UiRect::all(px(1)),
                justify_content: JustifyContent::Center,
                flex_shrink: 0.,
                ..default()
            },
            BackgroundColor(Color::srgb(0.09, 0.13, 0.18)),
            BorderColor::all(Color::srgb(0.24, 0.31, 0.38)),
            ChildOf(parent),
        ))
        .id();
    text(commands, entity, caption, 12.)
}
fn toggle(commands: &mut Commands, parent: Entity, action: Action) {
    let label = button(commands, parent, "", action);
    commands.entity(label).insert(Label::Toggle(action));
}
fn panel(commands: &mut Commands, right: bool) -> Entity {
    commands
        .spawn((
            ScrollPanel(right),
            ScrollPosition::default(),
            Node {
                position_type: PositionType::Absolute,
                left: if right { Val::Auto } else { px(12) },
                right: if right { px(12) } else { Val::Auto },
                top: px(12),
                bottom: px(12),
                width: px(if right { RIGHT } else { LEFT }),
                padding: UiRect::all(px(14)),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.04, 0.06, 0.97)),
        ))
        .id()
}
fn heading(commands: &mut Commands, parent: Entity, value: &str) {
    let e = text(commands, parent, value, 13.);
    commands
        .entity(e)
        .insert(TextColor(Color::srgb(0.35, 0.76, 0.9)));
}
fn range(commands: &mut Commands, parent: Entity, control: Control, lab: &Lab) {
    let (caption, min, max, step, precision) = control.spec();
    let group = column(commands, parent);
    commands.entity(group).insert(Node {
        width: percent(100),
        flex_direction: FlexDirection::Column,
        row_gap: px(2),
        flex_shrink: 0.,
        ..default()
    });
    text(commands, group, caption, 12.);
    let r = row(commands, group);
    let value = control.value(lab);
    let label = text(commands, r, &control.caption(value), 15.);
    commands.entity(label).insert((
        Label::Value(control),
        Node {
            width: px(72),
            flex_shrink: 0.,
            ..default()
        },
        TextColor(Color::srgb(0.45, 0.85, 1.0)),
    ));
    let slider = commands
        .spawn((
            control,
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderRange::new(min, max),
            SliderValue(value),
            SliderStep(step),
            SliderPrecision(precision),
            Hovered::default(),
            Node {
                height: px(25),
                flex_grow: 1.,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                ..default()
            },
            ChildOf(r),
        ))
        .id();
    commands.spawn((
        Node {
            width: percent(100),
            height: px(5),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(Color::srgb(0.18, 0.25, 0.32)),
        Pickable::IGNORE,
        ChildOf(slider),
    ));
    let track = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(14),
                top: px(5),
                height: px(15),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(slider),
        ))
        .id();
    commands.spawn((
        SliderThumb,
        Thumb(slider),
        Node {
            position_type: PositionType::Absolute,
            width: px(14),
            height: px(15),
            left: percent((value - min) / (max - min) * 100.),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.3, 0.74, 0.95)),
        Pickable::IGNORE,
        ChildOf(track),
    ));
}
fn metric(commands: &mut Commands, parent: Entity, caption: &str, label: Label) {
    let r = row(commands, parent);
    let caption = text(commands, r, caption, 12.);
    commands.entity(caption).insert(Node {
        flex_grow: 1.,
        flex_shrink: 0.,
        ..default()
    });
    let value = text(commands, r, "", 13.);
    commands.entity(value).insert(label);
}
pub(super) fn setup(mut commands: Commands, lab: Res<Lab>) {
    let left = panel(&mut commands, false);
    heading(&mut commands, left, "ROBOT DAMAGE LAB");
    let name = text(&mut commands, left, "", 18.);
    commands.entity(name).insert(Label::Robot);
    text(&mut commands, left, "Search robots", 12.);
    commands.spawn((
        RobotSearch,
        TextInput,
        EditableText {
            allow_newlines: false,
            max_characters: Some(64),
            ..default()
        },
        TextLayout::no_wrap(),
        TextFont::from_font_size(14.),
        TextCursorStyle::default(),
        Node {
            min_height: px(30),
            padding: UiRect::axes(px(6), px(5)),
            border: px(1).all(),
            flex_shrink: 0.,
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(Color::srgb(0.06, 0.09, 0.12)),
        BorderColor::all(Color::srgb(0.24, 0.31, 0.38)),
        ChildOf(left),
    ));
    let r = row(&mut commands, left);
    button(&mut commands, r, "Clear", Action::Clear);
    button(&mut commands, r, "<", Action::Page(-1));
    button(&mut commands, r, ">", Action::Page(1));
    let page = text(&mut commands, left, "", 12.);
    commands.entity(page).insert(Label::Page);
    for i in 0..PAGE {
        let label = button(&mut commands, left, "", Action::Slot(i));
        commands.entity(label).insert(Label::Slot(i));
    }
    text(
        &mut commands,
        left,
        "LMB: hammer\nRMB: orbit / Shift+RMB: pan\nWheel: zoom / scroll panel\nTab: wire / B: internal boxes\nR: restore / Space: pause\n/ : search / Esc: quit",
        12.,
    );
    button(&mut commands, left, "QUIT", Action::Quit);
    let right = panel(&mut commands, true);
    heading(&mut commands, right, "DAMAGE LAB SETTINGS");
    let r = row(&mut commands, right);
    button(&mut commands, r, "DAMAGE", Action::Tab(0));
    button(&mut commands, r, "EFFECTS", Action::Tab(1));
    let damage = column(&mut commands, right);
    commands.entity(damage).insert(Section(0));
    heading(&mut commands, damage, "IMPACT");
    for control in [
        Control::Power,
        Control::Radius,
        Control::Depth,
        Control::Limit,
    ] {
        range(&mut commands, damage, control, &lab);
    }
    heading(&mut commands, damage, "SURFACE");
    range(&mut commands, damage, Control::Density, &lab);
    text(
        &mut commands,
        damage,
        "Density restores robot on release",
        11.,
    );
    let r = row(&mut commands, damage);
    toggle(&mut commands, r, Action::Shading);
    toggle(&mut commands, r, Action::Wear);
    let fx = column(&mut commands, right);
    commands.entity(fx).insert((
        Section(1),
        Node {
            display: Display::None,
            width: percent(100),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            flex_shrink: 0.,
            ..default()
        },
    ));
    let r = row(&mut commands, fx);
    toggle(&mut commands, r, Action::Particles);
    button(&mut commands, r, "Clear particles", Action::ClearEffects);
    for control in [
        Control::Sparks,
        Control::Chips,
        Control::Speed,
        Control::Lifetime,
    ] {
        range(&mut commands, fx, control, &lab);
    }
    heading(&mut commands, fx, "GROUND DEBRIS");
    for control in [Control::ChipSize, Control::DebrisLimit] {
        range(&mut commands, fx, control, &lab);
    }
    button(
        &mut commands,
        fx,
        "CLEAR GROUND DEBRIS",
        Action::ClearDebris,
    );
    text(
        &mut commands,
        fx,
        "Fragments stay until cleared or replaced
by newer ones at the limit.",
        11.,
    );
    heading(&mut commands, fx, "PAINT LOSS / SCRATCHES");
    for control in [Control::WearStrength, Control::WearRadius] {
        range(&mut commands, fx, control, &lab);
    }
    text(
        &mut commands,
        fx,
        "Particle settings apply to the next hit.\nPaint visibility updates existing marks.",
        11.,
    );
    heading(&mut commands, right, "VIEW");
    let r = row(&mut commands, right);
    toggle(&mut commands, r, Action::Wire);
    toggle(&mut commands, r, Action::Boxes);
    let r = row(&mut commands, right);
    toggle(&mut commands, r, Action::Impact);
    toggle(&mut commands, r, Action::Pause);
    button(&mut commands, right, "RESTORE ROBOT", Action::Reset);
    heading(&mut commands, right, "PERFORMANCE");
    for (caption, label) in [
        ("Frame rate", Label::Fps),
        ("Rigid step", Label::Rigid),
        ("Dent + internal damage", Label::Damage),
        ("Impact effects (CPU)", Label::EffectsHit),
        ("Surface update (CPU)", Label::Surface),
        ("Airborne / update", Label::Fx),
        ("Ground debris", Label::Debris),
        ("Detached assemblies", Label::Detached),
        ("Vertices / triangles", Label::Mesh),
        ("Hammer hits", Label::Strikes),
    ] {
        metric(&mut commands, right, caption, label);
    }
    let components = text(&mut commands, right, "", 11.);
    commands.entity(components).insert(Label::Components);
    let status = text(&mut commands, right, "", 11.);
    commands.entity(status).insert(Label::Status);
}

pub(super) fn range_change(
    event: On<ValueChange<f32>>,
    ranges: Query<&Control>,
    mut lab: ResMut<Lab>,
    mut commands: Commands,
) {
    let Ok(&control) = ranges.get(event.source) else {
        return;
    };
    if !event.value.is_finite() {
        return;
    }
    let (_, min, max, _, precision) = control.spec();
    let scale = 10f32.powi(precision);
    let value = (event.value.clamp(min, max) * scale).round() / scale;
    match control {
        Control::Power => lab.power = value,
        Control::Radius => lab.radius = value,
        Control::Depth => lab.depth = value,
        Control::Limit => lab.max_depth = value,
        Control::Density => {
            lab.density_preview = value;
            if event.is_final {
                lab.density = value as usize;
            }
        }
        Control::Sparks => lab.effect_settings.sparks = value,
        Control::Chips => lab.effect_settings.chips = value,
        Control::ChipSize => lab.effect_settings.chip_scale = value,
        Control::DebrisLimit => {
            lab.effect_settings.debris_limit = value as usize;
            lab.effects.set_debris_limit(value as usize);
        }
        Control::Speed => lab.effect_settings.speed = value,
        Control::Lifetime => lab.effect_settings.lifetime = value,
        Control::WearStrength => lab.effect_settings.wear_strength = value,
        Control::WearRadius => lab.effect_settings.wear_radius = value,
    }
    commands.entity(event.source).insert(SliderValue(value));
    lab.revision += 1;
}
pub(super) fn range_visuals(
    mut commands: Commands,
    lab: Res<Lab>,
    ranges: Query<(&SliderValue, &SliderRange, &Control)>,
    mut thumbs: Query<(&Thumb, &mut Node)>,
    mut sections: Query<(&Section, &mut Node), Without<Thumb>>,
    mut labels: Query<(&Label, &mut Text)>,
) {
    for (thumb, mut node) in &mut thumbs {
        if let Ok((value, range, control)) = ranges.get(thumb.0) {
            let current = control.value(&lab);
            if value.0 != current {
                commands.entity(thumb.0).insert(SliderValue(current));
            }
            node.left = percent(range.thumb_position(current) * 100.);
        }
    }
    for (section, mut node) in &mut sections {
        node.display = if section.0 == lab.ui_tab {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (label, mut text) in &mut labels {
        if let Label::Value(control) = label {
            let next = control.caption(control.value(&lab));
            if text.0 != next {
                text.0 = next;
            }
        }
    }
}
pub(super) fn activate(
    event: On<Activate>,
    actions: Query<&Action>,
    mut search: Single<&mut EditableText, With<RobotSearch>>,
    mut focus: ResMut<InputFocus>,
    catalogue: Res<content::Catalogue>,
    mut lab: ResMut<Lab>,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
) {
    let Ok(action) = actions.get(event.entity) else {
        return;
    };
    match *action {
        Action::Slot(slot) => {
            let matches = catalogue.matching(&lab.filter);
            if let Some(&i) = matches.get(lab.page * PAGE + slot) {
                lab.selection = i;
                focus.clear();
            }
        }
        Action::Page(d) => {
            let pages = catalogue.matching(&lab.filter).len().div_ceil(PAGE).max(1);
            lab.page = (lab.page as i32 + d).rem_euclid(pages as i32) as usize;
        }
        Action::Clear => {
            search.clear();
            lab.filter.clear();
            lab.page = 0;
        }
        Action::Wire => {
            lab.wire = !lab.wire;
            if lab.wire {
                lab.boxes = true;
            }
        }
        Action::Shading => lab.smooth = !lab.smooth,
        Action::Boxes => lab.boxes = !lab.boxes,
        Action::Impact => lab.show_impact = !lab.show_impact,
        Action::Pause => lab.paused = !lab.paused,
        Action::Reset => lab.reset = true,
        Action::Tab(tab) => lab.ui_tab = tab,
        Action::Particles => {
            lab.effect_settings.enabled = !lab.effect_settings.enabled;
            if !lab.effect_settings.enabled {
                lab.effects.clear_airborne();
            }
        }
        Action::Wear => lab.effect_settings.wear = !lab.effect_settings.wear,
        Action::ClearEffects => lab.effects.clear_airborne(),
        Action::ClearDebris => lab.effects.clear_debris(),
        Action::Quit => {
            exit.write(crate::shutdown::ExitRequest);
        }
    }
    lab.revision += 1;
}
pub(super) fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    search: Single<(Entity, &EditableText), With<RobotSearch>>,
    mut focus: ResMut<InputFocus>,
    mut lab: ResMut<Lab>,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
) {
    let (field, text) = *search;
    if text.value() != lab.filter.as_str() {
        lab.filter = text.value().to_string();
        lab.page = 0;
    }
    let was_focused = lab.search_focus;
    lab.search_focus = focus.get() == Some(field);
    if lab.search_focus {
        if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Escape]) {
            focus.clear();
            lab.search_focus = false;
        }
    } else if keys.just_pressed(KeyCode::Slash) {
        focus.set(field, FocusCause::Navigated);
        lab.search_focus = true;
    } else {
        if !was_focused && keys.just_pressed(KeyCode::Escape) {
            exit.write(crate::shutdown::ExitRequest);
        }
        if keys.just_pressed(KeyCode::KeyR) {
            lab.reset = true;
        }
        if keys.just_pressed(KeyCode::Space) {
            lab.paused = !lab.paused;
        }
        if keys.just_pressed(KeyCode::Tab) {
            lab.wire = !lab.wire;
            if lab.wire {
                lab.boxes = true;
            }
            lab.revision += 1;
        }
        if keys.just_pressed(KeyCode::KeyB) {
            lab.boxes = !lab.boxes;
            lab.revision += 1;
        }
        if keys.just_pressed(KeyCode::KeyI) {
            lab.show_impact = !lab.show_impact;
            lab.revision += 1;
        }
    }
}

pub(super) fn over_ui(position: Vec2, size: Vec2) -> bool {
    position.x < LEFT + 24. || position.x > size.x - RIGHT - 24.
}
pub(super) fn scroll(
    window: Single<&Window>,
    wheel: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    mut panels: Query<(&ScrollPanel, &ComputedNode, &mut ScrollPosition)>,
) {
    let Some(p) = window.cursor_position() else {
        return;
    };
    for (side, node, mut position) in &mut panels {
        if (side.0 && p.x > window.width() - RIGHT - 24.) || (!side.0 && p.x < LEFT + 24.) {
            let max = (node.content_size().y - node.size().y).max(0.) * node.inverse_scale_factor;
            let step = if wheel.unit == bevy::input::mouse::MouseScrollUnit::Line {
                30.
            } else {
                1.
            };
            position.0.y = (position.0.y - wheel.delta.y * step).clamp(0., max);
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn refresh(
    time: Res<Time<Real>>,
    mut elapsed: Local<f32>,
    lab: Res<Lab>,
    catalogue: Res<content::Catalogue>,
    graphics: Res<render::Graphics>,
    effect_graphics: Res<effects::Graphics>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    mut labels: Query<(&Label, &mut Text)>,
    mut buttons: Query<(&Action, &mut BackgroundColor)>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 0.12 {
        return;
    }
    *elapsed = 0.;
    let matches = catalogue.matching(&lab.filter);
    let pages = matches.len().div_ceil(PAGE).max(1);
    for (label, mut text) in &mut labels {
        let value = match *label {
            Label::Value(_) => continue,
            Label::Robot => {
                let hp = &lab.active.loaded.content.health.parts[0];
                format!(
                    "{}  |  hull {:.0}/{:.0} HP",
                    catalogue.entries[lab.active.index].name, hp.current, hp.maximum
                )
            }
            Label::Slot(slot) => matches
                .get(lab.page * PAGE + slot)
                .map_or("—".into(), |&i| {
                    format!(
                        "{} {}",
                        if i == lab.active.index { ">" } else { " " },
                        catalogue.entries[i].name
                    )
                }),
            Label::Page => format!("{} robots / {} of {}", matches.len(), lab.page + 1, pages),
            Label::Toggle(action) => match action {
                Action::Shading => {
                    format!("Shading: {}", if lab.smooth { "Smooth" } else { "Flat" })
                }
                Action::Wire => format!("Wire: {}", on(lab.wire)),
                Action::Boxes => format!("Internal boxes: {}", on(lab.boxes)),
                Action::Impact => format!("Impact ring: {}", on(lab.show_impact)),
                Action::Pause => {
                    if lab.paused {
                        "RESUME".into()
                    } else {
                        "PAUSE".into()
                    }
                }
                Action::Particles => format!("Particles: {}", on(lab.effect_settings.enabled)),
                Action::Wear => format!("Paint loss: {}", on(lab.effect_settings.wear)),
                _ => String::new(),
            },
            Label::Fps => format!(
                "{:.0} FPS",
                diagnostics
                    .get(&FrameTimeDiagnosticsPlugin::FPS)
                    .and_then(|d| d.smoothed())
                    .unwrap_or(0.)
            ),
            Label::Rigid => format!("{:.3} ms", lab.active.physics.step_ms),
            Label::Damage => format!("{:.3} ms", lab.damage_ms),
            Label::EffectsHit => format!("{:.3} ms", lab.effects_hit_ms),
            Label::Surface => format!("{:.3} ms", graphics.surface_ms),
            Label::Fx => format!(
                "{} / {:.3} ms",
                lab.effects.count(),
                lab.effects.frame_ms + effect_graphics.draw_ms
            ),
            Label::Debris => format!(
                "{} / {}",
                lab.effects.debris_count(),
                lab.effect_settings.debris_limit
            ),
            Label::Detached => format!(
                "{} / {} parts",
                lab.active.physics.detached.len(),
                lab.active
                    .physics
                    .detached
                    .iter()
                    .map(|g| g.parts.len())
                    .sum::<usize>()
            ),
            Label::Mesh => format!(
                "{} / {}",
                lab.active.loaded.shell.positions.len(),
                lab.active.loaded.shell.triangles.len()
            ),
            Label::Strikes => lab.strikes.to_string(),
            Label::Components => {
                let mut parts: Vec<_> = lab
                    .active
                    .loaded
                    .boxes
                    .iter()
                    .filter(|b| b.damage > 0. || !lab.active.physics.attached(b.component))
                    .collect();
                parts.sort_by(|a, b| b.damage.total_cmp(&a.damage));
                parts
                    .iter()
                    .take(5)
                    .map(|b| {
                        format!(
                            "{}: {:.0}/{:.0} HP{}",
                            b.name
                                .rsplit(['\\', '/'])
                                .next()
                                .unwrap_or(&b.name)
                                .trim_end_matches(".txt"),
                            lab.active.loaded.content.health.parts[b.component].current,
                            lab.active.loaded.content.health.parts[b.component].maximum,
                            if lab.active.physics.attached(b.component) {
                                ""
                            } else {
                                " [off]"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Label::Status => lab.status.clone(),
        };
        if text.0 != value {
            text.0 = value;
        }
    }
    for (action, mut color) in &mut buttons {
        let active = match *action {
            Action::Tab(tab) => lab.ui_tab == tab,
            Action::Slot(slot) => matches.get(lab.page * PAGE + slot) == Some(&lab.active.index),
            _ => false,
        };
        color.0 = if active {
            Color::srgb(0.12, 0.31, 0.42)
        } else {
            Color::srgb(0.09, 0.13, 0.18)
        };
    }
}
fn on(value: bool) -> &'static str {
    if value { "ON" } else { "OFF" }
}
