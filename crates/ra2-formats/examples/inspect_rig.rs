fn main() {
    let path = std::env::args().nth(1).expect("model.gmf");
    let imported = ra2_formats::ModelImporters::with_ra2()
        .decode(&std::fs::read(path).unwrap())
        .unwrap();
    let m = imported.model;
    for (i, mat) in &m.materials {
        println!(
            "Material {i} {} {:?}",
            mat.name,
            mat.texture.as_ref().map(|t| &t.file)
        );
    }
    println!("Bounds {:?}", m.bounds());
    for b in m.all_bodies() {
        println!(
            "Body {} mass={} disabled={} pos={:?} proxy={} display={} use={} geom={:?} children={:?}",
            b.name,
            b.mass,
            b.disabled,
            b.transform.w_axis,
            b.proxy,
            b.display_proxy,
            b.use_display_proxy,
            b.geometry,
            b.children.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }
    for a in &m.attachments {
        println!(
            "Socket {} props={:?} position={:?} Z={:?}",
            a.name, a.properties, a.transform.w_axis, a.transform.z_axis
        );
    }
    println!("Joints {:#?}", m.joints);
    println!(
        "Objects {:?}",
        m.objects
            .iter()
            .map(|o| (&o.name, &o.parent, o.visible))
            .collect::<Vec<_>>()
    );
    println!(
        "Extensions {:?}",
        imported
            .report
            .extensions
            .iter()
            .map(|e| (e.node_type, e.bytes.len()))
            .collect::<Vec<_>>()
    );
}
