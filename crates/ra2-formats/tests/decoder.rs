//! Synthetic fixtures: no proprietary game assets needed for the default test suite.
use arena_core::{CollisionGeometry, Vec3};
use ra2_formats::{GmfDecoder, ModelDecoder};

fn int(out: &mut Vec<u8>, n: u32) {
    out.extend(n.to_le_bytes());
}
fn float(out: &mut Vec<u8>, n: f32) {
    out.extend(n.to_le_bytes());
}
fn string(out: &mut Vec<u8>, s: &str) {
    int(out, s.len() as u32 + 1);
    out.extend(s.bytes());
    out.push(0);
}
fn chunk(id: u32, flags: u32, data: Vec<u8>) -> Vec<u8> {
    let mut out = vec![];
    int(&mut out, id);
    int(&mut out, flags);
    int(&mut out, data.len() as u32);
    out.extend(data);
    out
}
fn tm(name: &str) -> Vec<u8> {
    let mut out = vec![];
    string(&mut out, name);
    for f in [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 3.0] {
        float(&mut out, f);
    }
    chunk(17, 2, out)
}
fn fixture() -> Vec<u8> {
    let mut out = b"GMI".to_vec();
    for n in [3, 1, 0, 15] {
        int(&mut out, n);
    }
    let mut scene = vec![];
    string(&mut scene, "fixture.max");
    for _ in 0..6 {
        int(&mut scene, 0);
    }
    out.extend(chunk(1, 2, scene));

    let mut texture = vec![];
    for s in ["diffuse", "Bitmap", "fixture.bmp"] {
        string(&mut texture, s);
    }
    float(&mut texture, 1.0);
    int(&mut texture, 1);
    int(&mut texture, 4);
    for f in [0.0, 0.0, 2.0, 3.0, 0.5, 0.0, 0.0, 0.0, 0.0] {
        float(&mut texture, f);
    }
    int(&mut texture, 1);
    float(&mut texture, 0.0);
    for n in [0, 0, 0, 1, 0] {
        int(&mut texture, n);
    }
    let mut textures = vec![];
    int(&mut textures, 1);
    textures.extend(chunk(15, 4, texture));
    let mut material = vec![];
    int(&mut material, 0);
    string(&mut material, "test");
    string(&mut material, "Standard");
    for n in [0, 0x00ffffff, 0] {
        int(&mut material, n);
    }
    for _ in 0..4 {
        float(&mut material, 0.0);
    }
    int(&mut material, 12);
    float(&mut material, 0.0);
    float(&mut material, 0.0);
    int(&mut material, 0);
    int(&mut material, 1);
    int(&mut material, 1);
    material.extend(chunk(14, 2, textures));
    int(&mut material, 0);
    let mut materials = vec![];
    int(&mut materials, 1);
    materials.extend(chunk(8, 2, material));
    out.extend(chunk(7, 2, materials));

    let mut mesh = vec![];
    for n in [0, 3, 1, 3, 0, 0] {
        int(&mut mesh, n);
    }
    for f in [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
        float(&mut mesh, f);
    }
    for n in [0, 1, 2, 0] {
        int(&mut mesh, n);
    }
    // A non-finite, unused W coordinate is present in shipped GMI files as well.
    for f in [0.0, 0.0, f32::NAN, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        float(&mut mesh, f);
    }
    for n in [0, 1, 2, 0] {
        int(&mut mesh, n);
    } // UV face + no additional channels
    for _ in 0..4 {
        for f in [0.0, 1.0, 0.0] {
            float(&mut mesh, f);
        }
    }
    int(&mut mesh, 1);
    let mut object = vec![];
    string(&mut object, "triangle");
    string(&mut object, "");
    object.push(0);
    object.extend(tm("triangle"));
    let mut mesh = chunk(16, 4, mesh);
    // Known exporter bug: declared record length is too short. Schema still ends exactly.
    let length = u32::from_le_bytes(mesh[8..12].try_into().unwrap());
    mesh[8..12].copy_from_slice(&(length - 7).to_le_bytes());
    object.extend(mesh);

    let mut attach = vec![];
    string(&mut attach, "Point01");
    attach.extend(tm("Point01"));
    string(&mut attach, "Type = Attach\nID = 1\nAttach = Axle_F\n");
    let mut body = vec![];
    string(&mut body, "triangle");
    for f in [2.0, 0.1, 0.8, 0.5] {
        float(&mut body, f);
    }
    int(&mut body, 0);
    int(&mut body, 1);
    string(&mut body, "(null)");
    body.extend([0, 0, 1]);
    string(&mut body, "(null)");
    body.extend(tm("triangle"));
    int(&mut body, 0);
    int(&mut body, 0);
    let mut bodies = vec![];
    int(&mut bodies, 1);
    bodies.extend(chunk(32, 4, body));
    let mut collection = vec![];
    string(&mut collection, "bodies");
    int(&mut collection, 0);
    int(&mut collection, 0);
    collection.extend(chunk(33, 2, bodies));
    let mut objects = vec![];
    int(&mut objects, 3);
    objects.extend(chunk(2, 4, object));
    objects.extend(chunk(21, 2, attach));
    objects.extend(chunk(31, 4, collection));
    out.extend(chunk(18, 2, objects));
    out
}

#[test]
fn binary_geometry_transforms_uv_materials_physics_and_attachments() {
    let imported = GmfDecoder.decode(&fixture()).unwrap();
    let model = imported.model;
    let object = &model.objects[0];
    assert_eq!(object.vertices[2], Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(
        object.positions().next().unwrap(),
        Vec3::new(2.0, 0.0, -3.0)
    );
    assert_eq!(object.faces[0].uv, [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]]);
    assert_eq!(
        model.materials[&0]
            .texture
            .as_ref()
            .unwrap()
            .scale
            .to_array(),
        [2.0, 3.0]
    );
    assert_eq!(model.bodies[0].mass, 2.0);
    assert_eq!(model.bodies[0].geometry, CollisionGeometry::Convex);
    assert_eq!(model.attachments[0].properties["attach"], "Axle_F");
    assert!(
        imported
            .report
            .warnings
            .iter()
            .any(|w| w.contains("consumed"))
    );
}

#[test]
fn every_truncated_prefix_fails_without_panicking() {
    let bytes = fixture();
    for end in 0..bytes.len() {
        assert!(
            GmfDecoder.decode(&bytes[..end]).is_err(),
            "Accepted prefix {end}"
        );
    }
}

#[test]
fn rejects_oversized_counts_and_nonfinite_positions() {
    let mut bytes = fixture();
    let signature = [16u32.to_le_bytes(), 4u32.to_le_bytes()].concat();
    let mesh = bytes.windows(8).position(|w| w == signature).unwrap();
    bytes[mesh + 16..mesh + 20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(GmfDecoder.decode(&bytes).unwrap_err().contains("count"));
    let mut bytes = fixture();
    bytes[mesh + 36..mesh + 40].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        GmfDecoder
            .decode(&bytes)
            .unwrap_err()
            .contains("Non-finite")
    );
}

#[test]
fn rejects_unknown_record_and_unsupported_version() {
    let mut bytes = fixture();
    bytes[3..7].copy_from_slice(&4u32.to_le_bytes());
    assert!(GmfDecoder.decode(&bytes).is_err());
    let mut bytes = fixture();
    let marker = [18u32.to_le_bytes(), 2u32.to_le_bytes()].concat();
    let list = bytes.windows(8).position(|w| w == marker).unwrap();
    bytes[list + 16..list + 20].copy_from_slice(&999u32.to_le_bytes());
    assert!(
        GmfDecoder
            .decode(&bytes)
            .unwrap_err()
            .contains("Unsupported object record 999")
    );
}
