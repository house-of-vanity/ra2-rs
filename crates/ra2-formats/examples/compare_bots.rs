//! Semantic check after importing and saving an editor file in the original game.
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(args.len(), 2, "compare_bots ours.bot original-resave.bot");
    let read = |p: &str| ra2_formats::bot::decode(&std::fs::read(p).unwrap()).unwrap();
    let a = read(&args[0]);
    let b = read(&args[1]);
    assert_eq!(a.name, b.name);
    assert_eq!(a.weight_class, b.weight_class);
    assert_eq!(a.components.len(), b.components.len());
    for (i, (a, b)) in a.components.iter().zip(&b.components).enumerate() {
        assert_eq!(
            (a.parent, a.parent_mount, a.own_mount),
            (b.parent, b.parent_mount, b.own_mount),
            "part {i}"
        );
        assert_eq!(a.class, b.class);
        assert_eq!(
            a.definition.to_ascii_lowercase().replace('\\', "/"),
            b.definition.to_ascii_lowercase().replace('\\', "/")
        );
        assert!(
            a.mounting
                .matrix()
                .abs_diff_eq(b.mounting.matrix(), 0.00005),
            "mount matrix {i}"
        );
        assert_eq!(a.settings, b.settings);
    }
    assert_eq!(a.chassis.bottom, b.chassis.bottom);
    assert_eq!(a.chassis.top, b.chassis.top);
    assert!((a.chassis.height - b.chassis.height).abs() < 0.00001);
    assert_eq!(a.chassis.armour, b.chassis.armour);
    assert_eq!(a.chassis.face_count, b.chassis.face_count);
    assert!((a.chassis.surface_density - b.chassis.surface_density).abs() < 0.0001);
    assert!(
        (a.chassis.hardness - b.chassis.hardness).abs() < 0.0001,
        "Armor hardness differs: {} / {}",
        a.chassis.hardness,
        b.chassis.hardness
    );
    assert_eq!(a.chassis.armour_flags, b.chassis.armour_flags);
    for (a, b) in a
        .chassis
        .material_parameters
        .iter()
        .zip(b.chassis.material_parameters)
    {
        assert!((a - b).abs() < 0.0001);
    }
    let (am, bm) = (
        a.chassis.model.as_ref().unwrap(),
        b.chassis.model.as_ref().unwrap(),
    );
    assert_eq!(am.objects.len(), bm.objects.len());
    for (a, b) in am.objects.iter().zip(&bm.objects) {
        assert_eq!(a.vertices.len(), b.vertices.len());
        assert_eq!(a.faces.len(), b.faces.len());
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            assert!(a.abs_diff_eq(*b, 0.00001));
        }
    }

    let pixels = |data: &Option<Vec<u8>>| {
        data.as_deref()
            .map(|b| arena_core::editor::paint::rgba(b).unwrap().2)
    };
    let (ap, bp) = (
        pixels(&a.chassis.texture).unwrap(),
        pixels(&b.chassis.texture).unwrap(),
    );
    let difference = ap
        .iter()
        .zip(&bp)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    println!(
        "Native paint conversion: max channel delta {difference}, first RGB {:?} -> {:?}",
        &ap[..3],
        &bp[..3]
    );
    assert!(ap == bp, "Paint pixels must survive native resave");
    assert_eq!(
        a.chassis.paint_layout.rectangles,
        b.chassis.paint_layout.rectangles
    );
    assert_eq!(
        a.chassis.paint_layout.face_groups,
        b.chassis.paint_layout.face_groups
    );
    assert_eq!(
        a.chassis.paint_layout.top_corners,
        b.chassis.paint_layout.top_corners
    );
    assert_eq!(a.controllers.len(), b.controllers.len());
    for (a, b) in a.controllers.iter().zip(&b.controllers) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.position, b.position);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.keys, b.keys);
    }
    assert_eq!(a.bindings.len(), b.bindings.len());
    for (a, b) in a.bindings.iter().zip(&b.bindings) {
        assert_eq!(a.component, b.component);
        assert_eq!(a.action, b.action);
        assert_eq!(a.controller, b.controller);
        assert_eq!(a.key, b.key);
    }
    println!(
        "PASS: original engine imported/resaved {} parts, {} controls, {} wires; chassis, mounting frames, paint pixels and native paint metadata agree",
        a.components.len(),
        a.controllers.len(),
        a.bindings.len()
    );
}
