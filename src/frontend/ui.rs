use super::{Draft, MenuCamera, Navigation, Page, art::MenuAssets};
use crate::settings::EngineSettings;
use bevy::{
    picking::hover::Hovered,
    prelude::*,
    ui::{Pressed, widget::NodeImageMode},
    ui_widgets::{Activate, Button},
};

#[derive(Component)]
pub struct Screen(pub Page);
#[derive(Component)]
pub struct Placed {
    anchor: Vec2,
    rect: Vec4,
}
#[derive(Component)]
pub(super) struct ScaledFont(f32);
#[derive(Component)]
pub(super) struct SettingsPanel;
#[derive(Component)]
pub(super) struct StatusText;
#[derive(Component)]
pub(super) struct MenuVersion;
#[derive(Resource, Default)]
pub struct Status(pub String);

#[derive(Clone, Copy, Debug)]
pub enum Field {
    Fullscreen,
    Resolution,
    Vsync,
    Msaa,
    Shadows,
    Diagnostics,
    Volume,
}
#[derive(Component)]
pub(super) struct ValueText(Field);
#[derive(Component, Clone, Copy, Debug)]
pub enum Action {
    Placeholder,
    Workshop,
    Resume,
    Exhibition,
    Multiplayer,
    HostPause,
    Settings,
    Back,
    Main,
    Quit,
    Apply,
    Defaults,
    Change(Field, i32),
}
#[derive(Component)]
pub(super) struct OriginalButton(f32);

fn text(commands: &mut Commands, parent: Entity, caption: &str, size: f32, color: Color) -> Entity {
    commands
        .spawn((
            Text::new(caption),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            ChildOf(parent),
        ))
        .id()
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    caption: &str,
    action: Action,
    node: Node,
) -> Entity {
    let entity = commands
        .spawn((
            Button,
            Hovered::default(),
            action,
            node,
            BackgroundColor(Color::srgba(0.07, 0.085, 0.10, 0.96)),
            BorderColor::all(Color::srgb(0.58, 0.61, 0.62)),
            ChildOf(parent),
        ))
        .id();
    text(commands, entity, caption, 16.0, Color::WHITE);
    entity
}
fn button_node(width: f32, height: f32) -> Node {
    Node {
        width: px(width),
        height: px(height),
        border: UiRect::all(px(1)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}
fn place(commands: &mut Commands, entity: Entity, anchor: Vec2, rect: (f32, f32, f32, f32)) {
    commands.entity(entity).insert(Placed {
        anchor,
        rect: Vec4::new(rect.0, rect.1, rect.2, rect.3),
    });
}
fn image(
    commands: &mut Commands,
    assets: &MenuAssets,
    parent: Entity,
    name: &str,
    anchor: Vec2,
    rect: (f32, f32, f32, f32),
) {
    if let Some(handle) = assets.images.get(name) {
        let entity = commands
            .spawn((
                ImageNode::new(handle.clone()).with_mode(NodeImageMode::Stretch),
                Node::default(),
                ChildOf(parent),
            ))
            .id();
        place(commands, entity, anchor, rect);
    }
}

pub(super) fn setup(mut commands: Commands, assets: Res<MenuAssets>) {
    commands.init_resource::<Status>();
    let camera = commands
        .spawn((
            MenuCamera,
            Camera2d,
            IsDefaultUiCamera,
            Camera {
                order: 10,
                ..default()
            },
            Msaa::Off,
        ))
        .id();
    commands.spawn((
        MenuVersion,
        UiTargetCamera(camera),
        GlobalZIndex(30),
        bevy::picking::Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            right: px(9),
            bottom: px(5),
            ..default()
        },
        Text::new(concat!("v", env!("CARGO_PKG_VERSION"))),
        TextFont {
            font_size: FontSize::Px(12.0),
            ..default()
        },
        TextColor(Color::srgb(0.82, 0.82, 0.82)),
        bevy::ui::widget::TextShadow {
            offset: Vec2::ONE,
            color: Color::BLACK,
        },
    ));
    let root_node = || Node {
        position_type: PositionType::Absolute,
        width: percent(100),
        height: percent(100),
        ..default()
    };
    let main = commands
        .spawn((
            Screen(Page::Main),
            UiTargetCamera(camera),
            GlobalZIndex(10),
            root_node(),
            BackgroundColor(Color::srgb(0.12, 0.13, 0.13)),
        ))
        .id();
    if let Some(handle) = assets.images.get(&assets.theme.background) {
        commands.spawn((
            ImageNode::new(handle.clone()).with_mode(NodeImageMode::Stretch),
            root_node(),
            ChildOf(main),
        ));
    }
    for art in &assets.theme.artwork {
        image(
            &mut commands,
            &assets,
            main,
            &art.path,
            Vec2::splat(0.5),
            art.rect,
        );
    }
    image(
        &mut commands,
        &assets,
        main,
        &assets.theme.logo,
        Vec2::new(0.5, 0.07),
        (-256.0, 0.0, 512.0, 76.0),
    );
    for entry in &assets.theme.buttons {
        let entity = button(
            &mut commands,
            main,
            &entry.caption,
            if entry.caption == "TEAM HQ" {
                Action::Workshop
            } else if entry.caption == "MULTIPLAYER" {
                Action::Multiplayer
            } else if entry.caption == "EXHIBITION" {
                Action::Exhibition
            } else {
                Action::Placeholder
            },
            button_node(165.0, 26.0),
        );
        commands.entity(entity).insert(OriginalButton(20.0));
        place(&mut commands, entity, Vec2::splat(0.5), entry.rect);
    }
    // The original footer, including the little gears and rounded metal ends.
    let footer = commands
        .spawn((
            Node::default(),
            BackgroundColor(Color::srgba(0.35, 0.35, 0.35, 0.9)),
            ChildOf(main),
        ))
        .id();
    place(
        &mut commands,
        footer,
        Vec2::new(0.5, 1.0),
        (-103.0, -36.0, 206.0, 28.0),
    );
    for (file, x) in [("rounded_rectL.tga", -128.0), ("rounded_rectR.tga", 96.0)] {
        image(
            &mut commands,
            &assets,
            main,
            &format!("UI/common/{file}"),
            Vec2::new(0.5, 1.0),
            (x, -36.0, 32.0, 32.0),
        );
    }
    for x in [-128.0, 97.0] {
        image(
            &mut commands,
            &assets,
            main,
            "UI/common/little_gear.tga",
            Vec2::new(0.5, 1.0),
            (x, -38.0, 32.0, 32.0),
        );
    }
    for (caption, action, x, width) in [
        ("Options", Action::Settings, -96.0, 67.0),
        ("Credits", Action::Placeholder, -23.0, 72.0),
        ("Quit", Action::Quit, 55.0, 40.0),
    ] {
        let entity = button(
            &mut commands,
            main,
            caption,
            action,
            button_node(width, 19.0),
        );
        commands.entity(entity).insert(OriginalButton(14.0));
        place(
            &mut commands,
            entity,
            Vec2::new(0.5, 1.0),
            (x, -31.0, width, 19.0),
        );
    }
    let status = text(&mut commands, main, "", 12.0, Color::WHITE);
    commands
        .entity(status)
        .insert((StatusText, ScaledFont(12.0)));
    place(
        &mut commands,
        status,
        Vec2::new(0.5, 1.0),
        (-260.0, -64.0, 520.0, 20.0),
    );
    // A separately designed engine settings panel. Its layout does not depend on GIB.
    let settings = commands
        .spawn((
            Screen(Page::Settings),
            UiTargetCamera(camera),
            GlobalZIndex(20),
            root_node(),
            BackgroundColor(Color::srgba(0.015, 0.02, 0.025, 0.86)),
        ))
        .id();
    let panel = commands
        .spawn((
            SettingsPanel,
            Node {
                position_type: PositionType::Absolute,
                width: px(680),
                height: px(540),
                padding: UiRect::all(px(24)),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.055, 0.07, 0.085)),
            BorderColor::all(Color::srgb(0.8, 0.45, 0.12)),
            ChildOf(settings),
        ))
        .id();
    text(
        &mut commands,
        panel,
        "ENGINE SETTINGS",
        26.0,
        Color::srgb(1.0, 0.7, 0.28),
    );
    text(
        &mut commands,
        panel,
        "Apply changes without restarting the game.",
        14.0,
        Color::srgb(0.7, 0.76, 0.8),
    );
    for (field, label) in [
        (Field::Fullscreen, "Window mode"),
        (Field::Resolution, "Window resolution"),
        (Field::Vsync, "VSync"),
        (Field::Msaa, "Anti-aliasing"),
        (Field::Shadows, "Shadows"),
        (Field::Diagnostics, "Performance overlay"),
        (Field::Volume, "Menu music volume"),
    ] {
        let row = commands
            .spawn((
                Node {
                    height: px(36),
                    min_height: px(36),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                },
                ChildOf(panel),
            ))
            .id();
        text(
            &mut commands,
            row,
            label,
            16.0,
            Color::srgb(0.85, 0.89, 0.92),
        );
        let values = commands
            .spawn((
                Node {
                    width: px(310),
                    column_gap: px(8),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                },
                ChildOf(row),
            ))
            .id();
        button(
            &mut commands,
            values,
            "<",
            Action::Change(field, -1),
            button_node(32.0, 30.0),
        );
        let value = text(&mut commands, values, "", 15.0, Color::WHITE);
        commands.entity(value).insert(ValueText(field));
        button(
            &mut commands,
            values,
            ">",
            Action::Change(field, 1),
            button_node(32.0, 30.0),
        );
    }
    text(
        &mut commands,
        panel,
        "Fullscreen uses the desktop resolution (Wayland).",
        13.0,
        Color::srgb(0.7, 0.76, 0.8),
    );
    let note = text(&mut commands, panel, "", 13.0, Color::srgb(1.0, 0.7, 0.28));
    commands.entity(note).insert(StatusText);
    let actions = commands
        .spawn((
            Node {
                column_gap: px(12),
                margin: UiRect::top(px(6)),
                ..default()
            },
            ChildOf(panel),
        ))
        .id();
    for (caption, action) in [
        ("APPLY", Action::Apply),
        ("DEFAULTS", Action::Defaults),
        ("BACK", Action::Back),
    ] {
        button(
            &mut commands,
            actions,
            caption,
            action,
            button_node(140.0, 36.0),
        );
    }
    let pause = commands
        .spawn((
            Screen(Page::Pause),
            UiTargetCamera(camera),
            GlobalZIndex(20),
            Node {
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..root_node()
            },
            BackgroundColor(Color::srgba(0.015, 0.02, 0.025, 0.8)),
        ))
        .id();
    let pause_panel = commands
        .spawn((
            Node {
                width: px(300),
                padding: UiRect::all(px(24)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(18),
                ..default()
            },
            BackgroundColor(Color::srgb(0.055, 0.07, 0.085)),
            ChildOf(pause),
        ))
        .id();
    text(&mut commands, pause_panel, "PAUSED", 28.0, Color::WHITE);
    for (caption, action) in [
        ("RESUME", Action::Resume),
        ("PAUSE / UNPAUSE MATCH", Action::HostPause),
        ("SETTINGS", Action::Settings),
        ("MAIN MENU", Action::Main),
        ("QUIT", Action::Quit),
    ] {
        button(
            &mut commands,
            pause_panel,
            caption,
            action,
            button_node(240.0, 40.0),
        );
    }
}

pub(super) fn resize(
    windows: Query<&Window>,
    assets: Res<MenuAssets>,
    mut last_size: Local<Option<Vec2>>,
    mut placed: Query<(&Placed, &mut Node), Without<SettingsPanel>>,
    mut panels: Query<(&mut Node, &mut UiTransform), With<SettingsPanel>>,
    mut fonts: Query<(Option<&ScaledFont>, &mut TextFont)>,
    original: Query<(&OriginalButton, &Children)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if *last_size == Some(size) {
        return;
    }
    *last_size = Some(size);
    let scale =
        (size.x / assets.theme.reference_size.0).min(size.y / assets.theme.reference_size.1);
    for (placed, mut node) in &mut placed {
        node.position_type = PositionType::Absolute;
        node.left = px(size.x * placed.anchor.x + placed.rect.x * scale);
        node.top = px(size.y * placed.anchor.y + placed.rect.y * scale);
        node.width = px(placed.rect.z * scale);
        node.height = px(placed.rect.w * scale);
    }
    for (mut node, mut transform) in &mut panels {
        node.left = px((size.x - 680.0) / 2.0);
        node.top = px((size.y - 540.0) / 2.0);
        transform.scale = Vec2::splat(
            ((size.x - 24.0) / 680.0)
                .min((size.y - 24.0) / 540.0)
                .min(1.0),
        );
    }
    for (font, mut text_font) in &mut fonts {
        if let Some(font) = font {
            text_font.font_size = FontSize::Px(font.0 * scale);
        }
    }
    for (button, children) in &original {
        for child in children {
            if let Ok((_, mut text_font)) = fonts.get_mut(*child) {
                text_font.font_size = FontSize::Px(button.0 * scale);
            }
        }
    }
}

fn cycle<T: PartialEq + Copy>(current: T, choices: &[T], direction: i32) -> T {
    let index = choices.iter().position(|v| *v == current).unwrap_or(0) as i32;
    choices[(index + direction).rem_euclid(choices.len() as i32) as usize]
}
pub(super) fn change(settings: &mut EngineSettings, field: Field, direction: i32) {
    match field {
        Field::Fullscreen => settings.fullscreen = !settings.fullscreen,
        Field::Resolution if !settings.fullscreen => {
            let mut choices = vec![
                [800, 600],
                [1024, 768],
                [1280, 720],
                [1280, 800],
                [1366, 768],
                [1600, 900],
                [1920, 1080],
                [2560, 1440],
                [3840, 2160],
                settings.resolution,
            ];
            choices.sort();
            choices.dedup();
            settings.resolution = cycle(settings.resolution, &choices, direction);
        }
        Field::Resolution => {}
        Field::Vsync => settings.vsync = !settings.vsync,
        Field::Msaa => settings.msaa = cycle(settings.msaa, &[1, 2, 4, 8], direction),
        Field::Shadows => settings.shadows = !settings.shadows,
        Field::Diagnostics => settings.diagnostics = !settings.diagnostics,
        Field::Volume => {
            settings.music_volume =
                (settings.music_volume as i32 + direction * 10).clamp(0, 100) as u32
        }
    }
}

pub(super) fn dispatch(
    action: Action,
    nav: &mut Navigation,
    draft: &mut Draft,
    applied: &mut EngineSettings,
    status: &mut Status,
) -> bool {
    status.0.clear();
    match action {
        Action::Placeholder => status.0 = "This game mode is not implemented yet.".into(),
        Action::Workshop => nav.page = Page::TeamSelect,
        Action::Exhibition => nav.page = Page::Exhibition,
        Action::Multiplayer => nav.page = Page::Multiplayer,
        Action::HostPause => {}
        Action::Resume => {
            nav.page = Page::Match;
        }
        Action::Settings => {
            nav.back = nav.page;
            nav.page = Page::Settings;
            draft.0 = applied.clone();
        }
        Action::Back => nav.page = nav.back,
        Action::Main => nav.page = Page::Main,
        Action::Quit => return true,
        Action::Defaults => draft.0 = EngineSettings::default(),
        Action::Change(field, direction) => change(&mut draft.0, field, direction),
        Action::Apply => {
            *applied = draft.0.clone();
            status.0 = "Settings applied.".into();
        }
    }
    false
}

#[allow(clippy::type_complexity, reason = "Bevy UI state and style query")]
pub(super) fn interact(
    mut buttons: Query<
        (
            Has<Pressed>,
            &Hovered,
            Option<&OriginalButton>,
            &mut BackgroundColor,
        ),
        With<Button>,
    >,
) {
    for (pressed, hovered, original, mut color) in &mut buttons {
        let next = if pressed {
            Color::srgba(0.25, 0.3, 0.7, 0.9)
        } else if hovered.0 {
            Color::srgba(0.32, 0.35, 0.38, 0.9)
        } else if original.is_some() {
            Color::srgba(0.0, 0.0, 0.0, 0.5)
        } else {
            Color::srgba(0.07, 0.085, 0.10, 0.96)
        };
        if color.0 != next {
            color.0 = next;
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "UI activation observer dependencies"
)]
pub(super) fn activate(
    event: On<Activate>,
    actions: Query<&Action>,
    mut nav: ResMut<Navigation>,
    mut draft: ResMut<Draft>,
    mut applied: ResMut<EngineSettings>,
    mut status: ResMut<Status>,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
    mut sounds: ResMut<crate::ui_sound::Sounds>,
    mut net: ResMut<crate::multiplayer::Multiplayer>,
    battle: Res<crate::battle::Session>,
) {
    if let Ok(Action::HostPause) = actions.get(event.entity)
        && battle.network
        && net.host
    {
        let tick = net.tick;
        let paused = !net.paused;
        net.pause(tick, paused);
    }
    if let Ok(action) = actions.get(event.entity)
        && dispatch(*action, &mut nav, &mut draft, &mut applied, &mut status)
    {
        exit.write(crate::shutdown::ExitRequest);
    }
    if actions.get(event.entity).is_ok() {
        sounds.play("Sounds/button_click.wav");
    }
}

pub(super) fn refresh(
    nav: Res<Navigation>,
    draft: Res<Draft>,
    status: Res<Status>,
    mut roots: Query<(&Screen, &mut Node)>,
    mut version: Single<&mut Visibility, With<MenuVersion>>,
    mut values: Query<(&ValueText, &mut Text), Without<StatusText>>,
    mut notes: Query<&mut Text, With<StatusText>>,
    mut host_pause: Query<(&Action, &mut Node), Without<Screen>>,
    battle: Res<crate::battle::Session>,
) {
    if nav.is_changed() {
        **version = if nav.page == Page::Match {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    for (action, mut node) in &mut host_pause {
        if matches!(action, Action::HostPause) {
            node.display = if battle.network && battle.authority {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    for (screen, mut node) in &mut roots {
        let visible = if screen.0 == Page::Main {
            nav.page == Page::Main || (nav.page == Page::Settings && nav.back == Page::Main)
        } else {
            nav.page == screen.0
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    if draft.is_changed() || nav.is_changed() {
        let on_off = |on| if on { "On".into() } else { "Off".into() };
        for (field, mut text) in &mut values {
            text.0 = match field.0 {
                Field::Fullscreen => {
                    if draft.0.fullscreen {
                        "Fullscreen".into()
                    } else {
                        "Windowed".into()
                    }
                }
                Field::Resolution => {
                    if draft.0.fullscreen {
                        "Desktop".into()
                    } else {
                        format!("{} x {}", draft.0.resolution[0], draft.0.resolution[1])
                    }
                }
                Field::Vsync => on_off(draft.0.vsync),
                Field::Shadows => on_off(draft.0.shadows),
                Field::Diagnostics => on_off(draft.0.diagnostics),
                Field::Msaa => {
                    if draft.0.msaa == 1 {
                        "Off".into()
                    } else {
                        format!("MSAA {}x", draft.0.msaa)
                    }
                }
                Field::Volume => format!("{}%", draft.0.music_volume),
            };
        }
    }
    if status.is_changed() {
        for mut text in &mut notes {
            text.0 = status.0.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn team_hq_opens_team_selection_before_workshop() {
        let mut nav = Navigation {
            page: Page::Main,
            back: Page::Main,
        };
        let mut applied = EngineSettings::default();
        let mut draft = Draft(applied.clone());
        dispatch(
            Action::Workshop,
            &mut nav,
            &mut draft,
            &mut applied,
            &mut Status::default(),
        );
        assert_eq!(nav.page, Page::TeamSelect);
    }
    #[test]
    fn cancelling_settings_keeps_applied_values_and_returns_to_pause() {
        let mut nav = Navigation {
            page: Page::Pause,
            back: Page::Main,
        };
        let mut applied = EngineSettings::default();
        let saved = applied.clone();
        let mut draft = Draft(applied.clone());
        let mut status = Status::default();
        dispatch(
            Action::Settings,
            &mut nav,
            &mut draft,
            &mut applied,
            &mut status,
        );
        dispatch(
            Action::Change(Field::Vsync, 1),
            &mut nav,
            &mut draft,
            &mut applied,
            &mut status,
        );
        assert_ne!(draft.0.vsync, applied.vsync);
        dispatch(
            Action::Back,
            &mut nav,
            &mut draft,
            &mut applied,
            &mut status,
        );
        assert_eq!(nav.page, Page::Pause);
        assert_eq!(applied, saved);
        dispatch(
            Action::Settings,
            &mut nav,
            &mut draft,
            &mut applied,
            &mut status,
        );
        assert_eq!(draft.0, saved);
    }
    #[test]
    fn fullscreen_keeps_window_size_and_volume_stays_bounded() {
        let mut s = EngineSettings {
            fullscreen: true,
            music_volume: 100,
            ..default()
        };
        let resolution = s.resolution;
        change(&mut s, Field::Resolution, 1);
        assert_eq!(s.resolution, resolution);
        change(&mut s, Field::Volume, 1);
        assert_eq!(s.music_volume, 100);
        s.music_volume = 0;
        change(&mut s, Field::Volume, -1);
        assert_eq!(s.music_volume, 0);
    }
}
