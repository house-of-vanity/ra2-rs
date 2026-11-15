//! Frame-rate damage feedback, independent of the bitmap HUD's update cadence.
use super::*;
use crate::{
    frontend::{Navigation, Page},
    legacy_ui::{self, Output, State},
};
use arena_core::health::DamageNumber;

pub(crate) fn color(victim: u32, source: Option<u32>, player: u32, age: f32) -> [f32; 4] {
    let alpha = ((2.15 - age) / 0.45).clamp(0., 1.);
    let rgb = if victim == player {
        [1., 0.12, 0.08]
    } else if source == Some(player) {
        [0.15, 1., 0.25]
    } else {
        [1., 0.85, 0.4]
    };
    [rgb[0], rgb[1], rgb[2], alpha]
}
fn labels(
    ui: &mut State,
    numbers: &[DamageNumber],
    victim: u32,
    player: u32,
    view: Mat4,
    fov: f32,
    aspect: f32,
    region: &str,
    overstep: f32,
) {
    let projection = bevy::math::proj::perspective(fov, aspect.max(0.1), 0.1, 200.);
    for n in numbers.iter().filter(|n| n.amount.round() >= 1.) {
        let age = n.age + overstep;
        let clip = projection * view * (n.position + Vec3::Y * (0.3 + age * 0.3)).extend(1.);
        if clip.w <= 0. {
            continue;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.x.abs() > 1. || ndc.y.abs() > 1. {
            continue;
        }
        ui.floating_text.push(legacy_ui::FloatingText {
            region: region.into(),
            point: Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5),
            offset: Vec2::new(
                if victim == player {
                    -(30. + age * 3.)
                } else if n.source == Some(player) {
                    30. + age * 3.
                } else {
                    0.
                },
                0.,
            ),
            text: format!("{:.0}", n.amount),
            color: color(victim, n.source, player, age),
            diagnostics: false,
        });
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn project(
    mut ui: ResMut<State>,
    output: Res<Output>,
    nav: Res<Navigation>,
    garage: Res<crate::garage::Garage>,
    battle: Option<Res<crate::battle::Session>>,
    editor: Res<crate::editor::Editor>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    windows: Query<&Window>,
    fixed: Res<Time<Fixed>>,
    network: Res<crate::multiplayer::Multiplayer>,
    prediction: Res<crate::multiplayer::prediction::Stats>,
) {
    ui.floating_text.clear();
    let overstep = fixed.overstep().as_secs_f32().min(crate::STEP as f32);
    if nav.page == Page::Match
        && let Some(session) = battle
        && let Some(player) = session
            .robots
            .iter()
            .find(|r| r.id as usize == session.local_slot + 1)
        && let (Ok((pose, projection)), Ok(window)) =
            (cameras.get(session.camera), windows.single())
    {
        let fov = if let Projection::Perspective(p) = projection {
            p.fov
        } else {
            std::f32::consts::FRAC_PI_4
        };
        if session.network && network.diagnostics {
            let local = if network.host {
                String::new()
            } else {
                format!(
                    "\nPRED p95 {:.1} cm  max {:.1} cm | pending {} replay {}\nCPU {:.2} ms/step  {:.2} ms/checkpoint | resync {}",
                    prediction.p95() * 100.,
                    prediction.maximum * 100.,
                    prediction.pending,
                    prediction.last_replay,
                    prediction.step_ms / prediction.steps.max(1) as f64,
                    prediction.cpu_ms / prediction.checkpoints.max(1) as f64,
                    prediction.hard_resyncs
                )
            };
            ui.floating_text.push(legacy_ui::FloatingText {
                region: "@screen".into(),
                point: Vec2::new(0.02, 0.71),
                offset: Vec2::ZERO,
                text: format!(
                    "NET {} ms | tick {}\nRX {} KiB  TX {} KiB{}",
                    network.rtt / 1000,
                    network.last_frame_tick.max(network.tick),
                    network.received / 1024,
                    network.sent / 1024,
                    local
                ),
                color: [0.4, 1., 0.5, 1.],
                diagnostics: true,
            });
        }
        // Match renders to the full window, including the 4:3 HUD's side margins.
        for robot in &session.robots {
            labels(
                &mut ui,
                &robot.damage.health.numbers,
                robot.id,
                player.id,
                pose.to_matrix().inverse(),
                fov,
                window.width() / window.height(),
                "@screen",
                overstep,
            );
        }
    } else if nav.page == Page::Workshop
        && editor.mode == crate::editor::Mode::Test
        && editor.dialog.is_none()
        && let Some(rect) = output.regions.get("test arena placeholder")
        && let Ok((pose, projection)) = cameras.get(garage.camera)
    {
        let fov = if let Projection::Perspective(p) = projection {
            p.fov
        } else {
            std::f32::consts::FRAC_PI_4
        };
        labels(
            &mut ui,
            &garage.damage.health.numbers,
            garage.id,
            garage.id,
            pose.to_matrix().inverse(),
            fov,
            rect.width() / rect.height().max(1.),
            "test arena placeholder",
            overstep,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_advances_smoothly_without_bitmap_revision_or_merge_delay() {
        let mut ui = State::default();
        let mut numbers = vec![DamageNumber {
            amount: 25.,
            position: Vec3::new(0., 0., -5.),
            age: 0.,
            source: Some(1),
        }];
        labels(
            &mut ui,
            &numbers,
            2,
            1,
            Mat4::IDENTITY,
            std::f32::consts::FRAC_PI_4,
            4. / 3.,
            "@screen",
            0.,
        );
        let first = ui.floating_text[0].point;
        assert_eq!(ui.floating_text[0].text, "25");
        ui.floating_text.clear();
        numbers[0].age = 1. / 240.;
        labels(
            &mut ui,
            &numbers,
            2,
            1,
            Mat4::IDENTITY,
            std::f32::consts::FRAC_PI_4,
            4. / 3.,
            "@screen",
            0.,
        );
        let moved = first.y - ui.floating_text[0].point.y;
        assert!(moved > 0. && moved < 0.001);
        assert_eq!(ui.revision, 0);
    }
    #[test]
    fn color_follows_real_player_and_hit_source() {
        assert_eq!(&color(1, Some(2), 1, 0.)[..3], &[1., 0.12, 0.08]);
        assert_eq!(&color(2, Some(1), 1, 0.)[..3], &[0.15, 1., 0.25]);
        assert_ne!(&color(2, None, 1, 0.)[..3], &[0.15, 1., 0.25]);
        assert_eq!(color(1, None, 1, 2.15)[3], 0.);
    }
}
