use super::*;
use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    picking::hover::Hovered,
    ui_widgets::{Activate, ActivateOnPress, Button},
};

#[derive(Component, Clone, Copy)]
pub(super) enum Action {
    Tool(tools::Tool),
    Quality(usize),
    Solver,
    Iterations(i32),
    Hz,
    Power(f32),
    Wire,
    Pause,
    Slow,
    Reset,
}
#[derive(Component)]
pub(super) struct Stats;
#[derive(Component)]
pub(super) struct Setting(Action);
const GOLD: Color = Color::srgb(1.0, 0.64, 0.16);

fn label(commands: &mut Commands, parent: Entity, text: &str, size: f32, color: Color) {
    commands.spawn((
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        ChildOf(parent),
    ));
}
fn row(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(6),
                align_items: AlignItems::Center,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    caption: &str,
    action: Action,
    width: f32,
    dynamic: bool,
) {
    let e = commands
        .spawn((
            Button,
            ActivateOnPress,
            Hovered::default(),
            action,
            Node {
                width: px(width),
                height: px(32),
                padding: UiRect::horizontal(px(8)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.09, 0.13, 0.18)),
            BorderColor::all(Color::srgb(0.24, 0.31, 0.38)),
            ChildOf(parent),
        ))
        .id();
    let text = commands
        .spawn((
            Text::new(caption),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::WHITE),
            ChildOf(e),
        ))
        .id();
    if dynamic {
        commands.entity(text).insert(Setting(action));
    }
}
pub(super) fn setup(mut commands: Commands) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(18),
                top: px(18),
                width: px(310),
                padding: UiRect::all(px(18)),
                row_gap: px(10),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.045, 0.065, 0.94)),
            BorderColor::all(Color::srgb(0.25, 0.32, 0.38)),
        ))
        .id();
    label(
        &mut commands,
        panel,
        env!("CARGO_PKG_NAME"),
        13.0,
        Color::srgb(0.45, 0.65, 0.76),
    );
    label(&mut commands, panel, "DESTRUCTION LAB", 23.0, GOLD);
    label(
        &mut commands,
        panel,
        "SCOUT / ORIGINAL PRACTICE ARENA",
        11.0,
        Color::srgb(0.65, 0.72, 0.77),
    );
    label(
        &mut commands,
        panel,
        "MESH / SOLVER CHANGES RESTORE ROBOT",
        12.0,
        GOLD,
    );
    let r = row(&mut commands, panel);
    for (i, title) in ["LOW", "MED", "HIGH", "ULTRA"].iter().enumerate() {
        button(&mut commands, r, title, Action::Quality(i), 64.0, false);
    }
    button(
        &mut commands,
        panel,
        "Solver: FEM [F]",
        Action::Solver,
        274.0,
        true,
    );
    let r = row(&mut commands, panel);
    button(&mut commands, r, "-", Action::Iterations(-1), 40.0, false);
    label(&mut commands, r, "SOLVER ITERATIONS", 12.0, Color::WHITE);
    button(&mut commands, r, "+", Action::Iterations(1), 40.0, false);
    button(
        &mut commands,
        panel,
        "Physics: 60 Hz",
        Action::Hz,
        274.0,
        true,
    );
    label(&mut commands, panel, "TOOL POWER", 12.0, GOLD);
    let r = row(&mut commands, panel);
    button(&mut commands, r, "- [", Action::Power(-0.25), 80.0, false);
    button(&mut commands, r, "+ ]", Action::Power(0.25), 80.0, false);
    button(&mut commands, r, "Wire [Tab]", Action::Wire, 100.0, true);
    let r = row(&mut commands, panel);
    button(&mut commands, r, "Pause", Action::Pause, 130.0, true);
    button(&mut commands, r, "Slow x0.2", Action::Slow, 138.0, true);
    button(
        &mut commands,
        panel,
        "RESTORE ROBOT [R]",
        Action::Reset,
        274.0,
        false,
    );
    commands.spawn((
        Stats,
        Text::new("Preparing physics…"),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::srgb(0.76, 0.85, 0.9)),
        Node {
            margin: UiRect::top(px(8)),
            ..default()
        },
        ChildOf(panel),
    ));
    let bottom = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(35),
                bottom: px(20),
                padding: UiRect::all(px(12)),
                column_gap: px(10),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.045, 0.065, 0.94)),
        ))
        .id();
    button(
        &mut commands,
        bottom,
        "1  IMPACT HAMMER",
        Action::Tool(tools::Tool::Hammer),
        185.0,
        false,
    );
    button(
        &mut commands,
        bottom,
        "2  CHAINSAW",
        Action::Tool(tools::Tool::Saw),
        160.0,
        false,
    );
    label(
        &mut commands,
        bottom,
        "LMB / use\nRMB / orbit  /  wheel / zoom\nQ/E / blade angle",
        12.0,
        Color::WHITE,
    );
}

pub(super) fn dispatch(action: Action, lab: &mut Lab) {
    match action {
        Action::Tool(tool) => {
            lab.tool = tool;
            lab.cooldown = 0.0;
        }
        Action::Quality(q) => {
            if lab.quality != q {
                lab.quality = q;
                lab.iterations = QUALITIES[q].iterations;
                lab.reset = true;
            }
        }
        Action::Solver => {
            lab.direct = !lab.direct;
            lab.reset = true;
        }
        Action::Iterations(delta) => {
            lab.iterations = (lab.iterations as i32 + delta).clamp(1, 12) as usize;
            for (_, body) in lab.physics.world.soft_bodies.iter_mut() {
                body.set_additional_pgs_iterations(lab.iterations);
            }
        }
        Action::Hz => {
            lab.hz = if lab.hz == 60 { 120 } else { 60 };
            lab.backlog = 0.0;
            lab.samples.clear();
        }
        Action::Power(delta) => lab.power = (lab.power + delta).clamp(0.25, 4.0),
        Action::Wire => lab.wire = !lab.wire,
        Action::Pause => lab.paused = !lab.paused,
        Action::Slow => lab.slow = !lab.slow,
        Action::Reset => lab.reset = true,
    }
}
pub(super) fn activate(
    event: On<Activate>,
    actions: Query<&Action>,
    mut lab: ResMut<Lab>,
    mut focus: ResMut<bevy::input_focus::InputFocus>,
) {
    if let Ok(action) = actions.get(event.entity) {
        dispatch(*action, &mut lab);
        focus.clear();
    }
}
pub(super) fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut lab: ResMut<Lab>,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
) {
    for (key, action) in [
        (KeyCode::Digit1, Action::Tool(tools::Tool::Hammer)),
        (KeyCode::Digit2, Action::Tool(tools::Tool::Saw)),
        (KeyCode::F1, Action::Quality(0)),
        (KeyCode::F2, Action::Quality(1)),
        (KeyCode::F3, Action::Quality(2)),
        (KeyCode::F4, Action::Quality(3)),
        (KeyCode::KeyF, Action::Solver),
        (KeyCode::Minus, Action::Iterations(-1)),
        (KeyCode::Equal, Action::Iterations(1)),
        (KeyCode::KeyT, Action::Hz),
        (KeyCode::BracketLeft, Action::Power(-0.25)),
        (KeyCode::BracketRight, Action::Power(0.25)),
        (KeyCode::Tab, Action::Wire),
        (KeyCode::Space, Action::Pause),
        (KeyCode::KeyO, Action::Slow),
        (KeyCode::KeyR, Action::Reset),
    ] {
        if keys.just_pressed(key) {
            dispatch(action, &mut lab);
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(crate::shutdown::ExitRequest);
    }
}
pub(super) fn over_ui(position: Vec2, size: Vec2) -> bool {
    (position.x < 338.0 && position.y < 850.0) || position.y > size.y - 92.0
}

#[allow(clippy::type_complexity)]
pub(super) fn refresh(
    time: Res<Time>,
    mut timer: Local<f32>,
    lab: Res<Lab>,
    diagnostics: Res<DiagnosticsStore>,
    mut stats: Query<&mut Text, (With<Stats>, Without<Setting>)>,
    mut values: Query<(&Setting, &mut Text), Without<Stats>>,
    mut buttons: Query<(&Action, &Hovered, &mut BackgroundColor)>,
) {
    for (action, hovered, mut color) in &mut buttons {
        let selected = match action {
            Action::Tool(t) => *t == lab.tool,
            Action::Quality(q) => *q == lab.quality,
            _ => false,
        };
        color.0 = if selected {
            Color::srgb(0.5, 0.28, 0.07)
        } else if hovered.0 {
            Color::srgb(0.18, 0.26, 0.32)
        } else {
            Color::srgb(0.09, 0.13, 0.18)
        };
    }
    for (setting, mut text) in &mut values {
        text.0 = match setting.0 {
            Action::Solver => format!(
                "Solver: {} [F]",
                if lab.direct {
                    "FEM / direct"
                } else {
                    "FEM / iterative"
                }
            ),
            Action::Hz => format!("Physics: {} Hz [T]", lab.hz),
            Action::Pause => if lab.paused {
                "Resume [Space]"
            } else {
                "Pause [Space]"
            }
            .into(),
            Action::Slow => format!("Slow x0.2 {} [O]", if lab.slow { "ON" } else { "OFF" }),
            Action::Wire => format!("Wire {}", if lab.wire { "ON" } else { "OFF" }),
            _ => continue,
        };
    }
    *timer += time.delta_secs();
    if *timer < 0.2 {
        return;
    }
    *timer = 0.0;
    let mut samples: Vec<_> = lab.samples.iter().copied().collect();
    samples.sort_by(f64::total_cmp);
    let median = samples.get(samples.len() / 2).copied().unwrap_or(0.0);
    let p95 = samples
        .get((samples.len() as f32 * 0.95) as usize)
        .copied()
        .unwrap_or(median);
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let (nodes, cells) = lab
        .physics
        .world
        .soft_bodies
        .iter()
        .fold((0, 0), |(n, c), (_, b)| {
            (n + b.num_particles(), c + b.cells().len())
        });
    let broken = lab.physics.mounts.iter().filter(|m| m.broken).count();
    let overloaded = if p95 > 1000.0 / lab.hz as f64 {
        "OVER BUDGET"
    } else {
        "WITHIN BUDGET"
    };
    for mut text in &mut stats {
        text.0 = format!(
            "{fps:.0} FPS  /  {}\n\nPhysics step    {median:.2} ms\n95th percentile {p95:.2} ms\nStep budget     {:.2} ms\nMesh update     {:.2} ms\nDropped sim     {:.2} s\n{overloaded}\n\n{nodes} nodes  /  {cells} tetrahedra\n{} fragments  /  {broken} broken mounts\n{} cuts  /  {} hammer strikes\n{} iterations  /  power {:.2}x\n\nF1-F4 / mesh detail\n-/+ / solver iterations\nSpace / pause   O / slow motion\nR / restore     Esc / exit\n\n{}",
            QUALITIES[lab.quality].name,
            1000.0 / lab.hz as f64,
            lab.render_ms,
            lab.dropped,
            lab.physics.world.soft_bodies.len(),
            lab.physics.cuts,
            lab.strikes,
            lab.iterations,
            lab.power,
            lab.status
        );
    }
}
