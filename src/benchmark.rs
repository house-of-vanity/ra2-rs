//! Short reproducible capture window, before screenshot readbacks disturb frame times.
use crate::{Options, fleet::FleetStats};
use bevy::{diagnostic::DiagnosticsStore, prelude::*};
use bevy_rapier3d::prelude::RapierDiagnosticsPlugin;

#[derive(Default)]
pub struct Capture {
    rows: Vec<[f64; 5]>,
    done: bool,
}

pub fn sample(
    options: Res<Options>,
    time: Res<Time<Real>>,
    diagnostics: Res<DiagnosticsStore>,
    stats: Res<FleetStats>,
    mut capture: Local<Capture>,
) {
    if !options.all_bots || !options.smoke_test || capture.done {
        return;
    }
    let seconds = time.elapsed_secs_f64();
    if (2.5..4.5).contains(&seconds) {
        let value = |path: &bevy::diagnostic::DiagnosticPath| {
            diagnostics.get(path).and_then(|d| d.value()).unwrap_or(0.0)
        };
        capture.rows.push([
            seconds,
            time.delta_secs_f64() * 1000.0,
            value(&RapierDiagnosticsPlugin::STEP_TIME),
            value(&RapierDiagnosticsPlugin::ACTIVE_BODIES),
            value(&RapierDiagnosticsPlugin::ACTIVE_CONTACT_PAIRS),
        ]);
    }
    if seconds > 4.7 && !capture.rows.is_empty() {
        capture.done = true;
        let mut csv = String::from("seconds,frame_ms,physics_step_ms,awake_bodies,contact_pairs\n");
        for r in &capture.rows {
            csv.push_str(&format!(
                "{:.6},{:.6},{:.6},{:.0},{:.0}\n",
                r[0], r[1], r[2], r[3], r[4]
            ));
        }
        let mode = if options.bot_physics {
            "rigid"
        } else {
            "render"
        };
        std::fs::create_dir_all("logs").expect("Create benchmark directory");
        let path = format!("logs/bots-{}-{mode}.csv", options.bot_copies);
        std::fs::write(&path, csv).expect("Write benchmark capture");
        let mut times: Vec<_> = capture.rows.iter().map(|r| r[1]).collect();
        times.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
        let step = capture.rows.iter().map(|r| r[2]).sum::<f64>() / times.len() as f64;
        println!(
            "BENCHMARK: {} robots, {} components, {mode}, {} build; {:.1} FPS, mean {:.3} ms, p95 {:.3} ms, physics step {:.3} ms; {} samples -> {path}",
            stats.robots,
            stats.components,
            if cfg!(debug_assertions) {
                "optimized-dev"
            } else {
                "release"
            },
            1000.0 / mean,
            mean,
            p95,
            step,
            times.len()
        );
    }
}
