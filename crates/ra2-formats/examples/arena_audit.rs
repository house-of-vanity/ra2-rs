use ra2_formats::ModelImporters;
fn main() {
    let root = std::env::args().nth(1).unwrap_or("../RobotArena2".into());
    for path in [
        "box/boxarena.gmf",
        "skull/skull.gmf",
        "octagon/octagon.gmf",
        "electric/electric.gmf",
        "bridge/bridge.gmf",
        "compressor/compressor.gmf",
        "parkinglot/parkinglot.gmf",
        "half/half.gmf",
        "kingofhill/king.gmf",
        "hilltop/hilltop.gmf",
        "tabletop/clawtop.gmf",
        "flextop/flextop.gmf",
    ] {
        let file = std::path::Path::new(&root).join("Arenas").join(path);
        let m = ModelImporters::with_ra2()
            .decode(&std::fs::read(file).unwrap())
            .unwrap()
            .model;
        println!("=== {path}");
        for b in m.all_bodies() {
            println!(
                "B {} m={} geom={:?} proxy={} display={} used={} disabled={} at={:?}",
                b.name,
                b.mass,
                b.geometry,
                b.proxy,
                b.display_proxy,
                b.use_display_proxy,
                b.disabled,
                b.transform.w_axis.truncate()
            );
        }
        for j in &m.joints {
            println!("J {:?}", j);
        }
        for a in &m.attachments {
            if a.properties
                .get("type")
                .is_some_and(|t| t.eq_ignore_ascii_case("startpoint"))
            {
                println!("S {} {:?}", a.name, a.transform.w_axis.truncate());
            }
        }
    }
}
