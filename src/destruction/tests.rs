use super::*;
#[test]
#[ignore = "Requires original Robot Arena 2 game data"]
fn scout_armour_hammer_and_cut() {
    let content =
        Content::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2"))
            .unwrap();
    check(&content).unwrap();
}

#[test]
#[ignore = "Requires original Robot Arena 2 game data"]
fn scout_all_mesh_and_fem_presets_settle() {
    let content =
        Content::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2"))
            .unwrap();
    check_presets(&content).unwrap();
}

#[test]
#[ignore = "Requires original Robot Arena 2 game data"]
fn local_saw_strokes_keep_real_fragments_and_native_paint() {
    let content =
        Content::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2"))
            .unwrap();
    let mut lab = build_physics(&content, 2, true, 4).unwrap();
    for _ in 0..90 {
        lab.step(1.0 / 60.0, 4);
    }
    for i in 0..18 {
        let origin = Vec3::new(-0.7 + i as f32 * 0.08, 3.0, 0.12);
        if let Some((_, point, normal)) = lab.hit(origin, -Vec3::Y) {
            lab.cut(point, normal, Vec3::Z, 1.0);
        }
        for _ in 0..4 {
            lab.step(1.0 / 60.0, 4);
        }
        assert!(!lab.unstable, "saw stroke {i} destabilized the specimen");
    }
    assert!(lab.cuts > 0);
    let paint = lab
        .world
        .soft_bodies
        .iter()
        .flat_map(|(_, b)| lab.shell.surface(b))
        .filter(|f| f.uv.is_some())
        .count();
    assert!(paint > 0, "cuts lost all native paint");
    println!(
        "Local saw: {} cuts, {} fragments, {paint} painted faces",
        lab.cuts,
        lab.world.soft_bodies.len()
    );
}
