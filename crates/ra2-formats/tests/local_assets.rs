//! Opt-in compatibility checks against the user's own game files, never bundled here.
use ra2_formats::{GmfDecoder, ModelDecoder};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    std::env::var_os("RA2_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../RobotArena2"))
}
fn read(path: &Path) -> ra2_formats::ImportedModel {
    GmfDecoder
        .decode(&std::fs::read(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}
fn scan(path: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gmf"))
        {
            out.push(path);
        }
    }
}

#[test]
#[ignore = "requires a local RA2 installation"]
fn decodes_entire_local_corpus() {
    let mut paths = vec![];
    scan(&root(), &mut paths);
    assert!(!paths.is_empty());
    for path in &paths {
        read(path);
    }
    println!("Decoded and validated {} local GMF assets", paths.len());
}

#[test]
#[ignore = "requires a local RA2 installation"]
fn native_bot_writer_preserves_local_editor_data() {
    fn find(dir: &Path, paths: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                find(&path, paths);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("bot"))
            {
                paths.push(path);
            }
        }
    }
    let mut paths = vec![];
    find(&root(), &mut paths);
    assert!(!paths.is_empty());
    for path in &paths {
        let source = ra2_formats::bot::decode(&std::fs::read(path).unwrap()).unwrap();
        let bytes =
            ra2_formats::bot::encode(&source).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let loaded = ra2_formats::bot::decode(&bytes).unwrap();
        assert_eq!(source.name, loaded.name);
        assert_eq!(source.components.len(), loaded.components.len());
        assert_eq!(source.snapshot, loaded.snapshot);
        assert_eq!(source.chassis.texture, loaded.chassis.texture);
        assert_eq!(
            source.chassis.paint_layout.rectangles,
            loaded.chassis.paint_layout.rectangles
        );
        assert_eq!(
            source.chassis.paint_layout.face_groups,
            loaded.chassis.paint_layout.face_groups
        );
        assert_eq!(
            source.chassis.paint_layout.top_corners,
            loaded.chassis.paint_layout.top_corners
        );
        assert_eq!(source.controllers.len(), loaded.controllers.len());
        assert_eq!(source.bindings.len(), loaded.bindings.len());
        for (before, after) in source.components.iter().zip(&loaded.components) {
            assert_eq!(before.parent, after.parent);
            assert!(
                before
                    .mounting
                    .matrix()
                    .abs_diff_eq(after.mounting.matrix(), 0.0001)
            );
        }
    }
    println!(
        "Native BOT reader/writer verified on {} original files",
        paths.len()
    );
}

#[test]
#[ignore = "requires a local RA2 installation"]
fn original_gui_widgets_and_fonts_decode() {
    fn scan(dir: &Path, count: &mut usize) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                scan(&p, count);
            } else if p
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("gib"))
            {
                ra2_formats::gib::decode(&std::fs::read(&p).unwrap())
                    .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                *count += 1;
            }
        }
    }
    let mut count = 0;
    scan(&root().join("UI"), &mut count);
    let archive =
        ra2_formats::archive::CfzArchive::decode(&std::fs::read(root().join("RA2.cfz")).unwrap())
            .unwrap();
    let mut fonts = 0;
    for (name, bytes) in &archive.files {
        if name.ends_with(".giff") {
            let font = ra2_formats::giff::decode(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(font.glyphs.len(), 95);
            fonts += 1;
        }
    }
    assert!(count > 0 && fonts > 0);
    println!("Decoded {count} original GIB layouts and {fonts} original bitmap fonts");
}

#[test]
#[ignore = "requires a local RA2 installation"]
fn binary_matches_ascii_geometry_and_available_uvs() {
    let root = root();
    for (binary, ascii) in [
        ("Arenas/box/boxarena.gmf", "Arenas/box/boxarena_txt.gmf"),
        ("Arenas/bridge/bridge.gmf", "Arenas/bridge/bridge_txt.gmf"),
        (
            "Components/cannon/cannon.gmf",
            "Components/cannon/cannon_txt.gmf",
        ),
    ] {
        let binary = read(&root.join(binary)).model;
        let ascii = read(&root.join(ascii)).model;
        assert_eq!(binary.materials.len(), ascii.materials.len());
        for object in ascii.objects.iter().filter(|o| !o.faces.is_empty()) {
            let other = binary.object(&object.name).unwrap();
            assert_eq!(other.faces.len(), object.faces.len(), "{}", object.name);
            assert_eq!(other.vertices.len(), object.vertices.len());
            for (a, b) in object.positions().zip(other.positions()) {
                assert!(a.distance(b) < 0.001, "{}: {a:?} != {b:?}", object.name);
            }
            for (a, b) in object.faces.iter().zip(&other.faces) {
                assert_eq!(a.vertices, b.vertices);
                assert_eq!(a.material, b.material);
                for i in 0..3 {
                    assert!(
                        a.normals[i].distance(b.normals[i]) < 0.001,
                        "{} normals",
                        object.name
                    );
                    // cannon_txt.gmf ships without UV data; the binary cannon has it.
                    // Compare every UV where the ASCII source actually defines a channel.
                    if object.uv_channels.contains_key(&1) {
                        assert!(
                            (a.uv[i][0] - b.uv[i][0]).abs() < 0.001
                                && (a.uv[i][1] - b.uv[i][1]).abs() < 0.001,
                            "{} UV: {:?} vs {:?}",
                            object.name,
                            a.uv[i],
                            b.uv[i]
                        );
                    }
                }
            }
        }
        assert_eq!(binary.all_bodies().len(), ascii.all_bodies().len());
        assert_eq!(binary.attachments.len(), ascii.attachments.len());
    }
}
