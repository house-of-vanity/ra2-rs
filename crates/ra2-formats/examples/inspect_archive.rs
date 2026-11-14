fn main() {
    let path = std::env::args().nth(1).expect("archive.cfz");
    let archive = ra2_formats::archive::CfzArchive::decode(&std::fs::read(path).unwrap()).unwrap();
    if let Some(entry) = std::env::args().nth(2) {
        println!("{}", String::from_utf8_lossy(archive.get(&entry).unwrap()));
        return;
    }
    for (key, name) in &archive.names {
        println!("{} {}", archive.files[key].len(), name);
    }
}
