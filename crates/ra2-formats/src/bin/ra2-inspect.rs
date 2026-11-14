//! Headless asset inspection; this binary does not link Bevy or Rapier.
use ra2_formats::ModelImporters;
use std::path::{Path, PathBuf};

fn files(path: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            files(&entry?.path(), output)?;
        }
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gmf"))
    {
        output.push(path.to_owned());
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Usage: ra2-inspect MODEL.gmf | DIRECTORY")?;
    let path = Path::new(&path);
    let mut paths = vec![];
    files(path, &mut paths).map_err(|e| e.to_string())?;
    if paths.is_empty() {
        return Err("No GMF files found".into());
    }
    paths.sort();
    let importers = ModelImporters::with_ra2();
    let mut failed = 0;
    let mut binary = 0;
    let mut text = 0;
    for path in &paths {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        match importers.decode(&bytes) {
            Ok(import) => {
                if bytes.starts_with(b"GMI") {
                    binary += 1;
                } else {
                    text += 1;
                }
                let model = &import.model;
                println!(
                    "OK {}: {} | {} objects, {} triangles, {} materials, {} bodies, {} attachments | {} compatibility notes, {} retained extension nodes",
                    path.display(),
                    import.report.format,
                    model.objects.len(),
                    model.objects.iter().map(|o| o.faces.len()).sum::<usize>(),
                    model.materials.len(),
                    model.all_bodies().len(),
                    model.attachments.len(),
                    import.report.warnings.len(),
                    import.report.extensions.len()
                );
                if paths.len() == 1 {
                    println!("Bounds: {:?}", model.bounds());
                    for a in &model.attachments {
                        println!("Attachment {}: {:?}", a.name, a.properties);
                    }
                    for note in &import.report.warnings {
                        println!("NOTE {note}");
                    }
                }
            }
            Err(error) => {
                failed += 1;
                eprintln!("FAIL {}: {error}", path.display());
            }
        }
    }
    println!(
        "{} files: {binary} GMI, {text} GMA, {failed} failed",
        paths.len()
    );
    if failed > 0 {
        return Err("Asset inspection failed".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
