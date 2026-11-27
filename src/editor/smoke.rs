//! Opt-in end-to-end workshop exercise using original widget actions.
use super::{
    Editor, Mode,
    catalog::Catalog,
    controls,
    viewport::{Pick, View},
};
use crate::{
    Options,
    legacy_ui::{Events, Output, State, UiEvent},
};
use bevy::{
    audio::AudioSinkPlayback,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};

#[derive(Default)]
pub struct Check {
    stage: usize,
    entered: f64,
}
#[allow(clippy::too_many_arguments)]
pub fn run(
    options: Res<Options>,
    time: Res<Time<Real>>,
    mut check: Local<Check>,
    mut editor: ResMut<Editor>,
    catalog: Res<Catalog>,
    view: Res<View>,
    mut ui: ResMut<State>,
    output: Res<Output>,
    mut events: ResMut<Events>,
    mut commands: Commands,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
    music: Query<&AudioSink, With<crate::frontend::WorkshopMusic>>,
) {
    if !options.editor_smoke_test
        || time.elapsed_secs_f64() - check.entered < if check.stage == 0 { 3.0 } else { 0.8 }
    {
        return;
    }
    let click = |events: &mut Events, name: &str| {
        events.0.push(UiEvent::Activate {
            name: name.into(),
            owner: String::new(),
            point: Vec2::ZERO,
        });
    };
    let shot = |commands: &mut Commands, name: &str| {
        std::fs::create_dir_all("logs").unwrap();
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!("logs/editor-{name}.png")));
    };
    match check.stage {
        0 => {
            let music = music
                .single()
                .expect("Workshop music must have an audio sink");
            assert!(!music.is_paused() && !music.empty());
            assert!(output.hits.iter().any(|h| h.name == "chassis button"));
            shot(&mut commands, "overview");
        }
        1 => click(&mut events, "chassis button"),
        2 => click(&mut events, "structure design button"),
        3 => click(&mut events, "rectangle checkbox"),
        4 => shot(&mut commands, "base"),
        5 => click(&mut events, "step 2 button"),
        6 => {
            assert!(editor.document.robot.chassis.model.is_some());
            shot(&mut commands, "top");
        }
        7 => click(&mut events, "finished button"),
        8 => click(&mut events, "armor button"),
        9 => shot(&mut commands, "armor"),
        10 => {
            click(&mut events, "armor:2");
            click(&mut events, "components button");
        }
        11..=14 => {
            let (class, pos) = match check.stage {
                11 => ("ControlBoard", Vec3::new(0.0, 0.0, 0.3)),
                12 => ("Battery", Vec3::new(0.0, 0.0, -0.3)),
                13 => ("SpinMotor", Vec3::new(-0.45, 0.0, 0.0)),
                _ => ("SpinMotor", Vec3::new(0.45, 0.0, 0.0)),
            };
            let asset = catalog
                .components
                .values()
                .find(|a| {
                    a.definition.value("base") == class
                        && (class != "Battery" || a.key.ends_with("/battpack1.txt"))
                        && !a.key.contains("/styles/")
                        && a.definition.value("hidden") != "1"
                })
                .unwrap();
            editor.attach_angle = if check.stage == 14 {
                std::f32::consts::PI
            } else {
                0.0
            };
            editor.component = asset.key.clone();
            editor.pending = Some(asset.key.clone());
            controls::attach(
                &mut editor,
                &catalog,
                &Pick {
                    attachment: None,
                    component: 0,
                    position: pos,
                    normal: Vec3::Y,
                    face: 0,
                    uv: Vec2::ZERO,
                },
                &view,
            )
            .unwrap();
        }
        15 => shot(&mut commands, "components"),
        16 => click(&mut events, "wiring button"),
        17 => click(&mut events, "new analog"),
        18 => {
            ui.values.insert("control name".into(), "Drive".into());
            editor.key_positive = 17;
            editor.key_negative = 31;
            click(&mut events, "ok button");
        }
        19 => {
            editor.selected = Some(3);
            editor.binding_actions = [1, 2];
            editor.dialog = Some(super::Dialog::Connection);
            editor.ui_revision += 1;
        }
        20 => shot(&mut commands, "wiring-dialog"),
        21 => click(&mut events, "ok button"),
        22 => {
            assert_eq!(editor.document.robot.bindings.len(), 2);
            shot(&mut commands, "wiring");
        }
        23 => click(&mut events, "paintshop button"),
        24 => {
            editor.color = [190, 40, 20, 255];
            click(&mut events, "paint entire");
        }
        25 => shot(&mut commands, "paint"),
        26 => click(&mut events, "overview button"),
        27 => {
            events.0.push(UiEvent::Value {
                name: "bot name textbox".into(),
                value: "ra2-rs compatibility check".into(),
            });
            click(&mut events, "snapshot button");
        }
        28 => {
            assert!(editor.document.robot.snapshot.is_some());
            controls::save(
                &mut editor,
                std::path::Path::new("logs/editor-compatibility.bot"),
            )
            .unwrap();
            let decoded =
                ra2_formats::bot::decode(&std::fs::read("logs/editor-compatibility.bot").unwrap())
                    .unwrap();
            assert_eq!(decoded.components.len(), 5);
            assert_eq!(decoded.controllers.len(), 1);
            assert_eq!(decoded.bindings.len(), 2);
            assert!(decoded.snapshot.is_some());
            assert_eq!(editor.mode, Mode::Overview);
            shot(&mut commands, "saved");
        }
        29 => {
            info!(
                "EDITOR CHECK PASSED: original GUI, chassis, 4 components, controller, wiring, paint, GPU snapshot, workshop audio and native BOT save/read"
            );
            exit.write(crate::shutdown::ExitRequest);
        }
        _ => (),
    }
    check.stage += 1;
    check.entered = time.elapsed_secs_f64();
}
