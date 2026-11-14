fn main() {
    let b = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let r = ra2_formats::ModelImporters::with_ra2().decode(&b).unwrap();
    for e in r.report.extensions.iter().filter(|e| e.node_type == 42) {
        for (i, c) in e.bytes.chunks(16).enumerate() {
            println!(
                "{:04x}: {:02x?} {}",
                i * 16,
                c,
                c.iter()
                    .map(|b| if b.is_ascii_graphic() {
                        *b as char
                    } else {
                        '.'
                    })
                    .collect::<String>()
            );
        }
    }
}
