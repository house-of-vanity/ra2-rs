use ra2_formats::{bot, components::ComponentCatalog};
use std::path::{Path, PathBuf};
fn collect(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for e in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bot")) {
            out.push(p);
        }
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("Usage: ra2-bots GAME_DIRECTORY")?,
    );
    let catalog = ComponentCatalog::from_cfz(
        &std::fs::read(root.join("Components.cfz")).map_err(|e| e.to_string())?,
    )?;
    println!("{} component definitions", catalog.definitions.len());
    let mut files = vec![];
    collect(&root, &mut files)?;
    files.sort();
    let mut failed = 0;
    let mut parts = 0;
    for p in &files {
        match bot::decode(&std::fs::read(p).map_err(|e| e.to_string())?) {
            Ok(bot) => {
                for c in bot.components.iter().skip(1) {
                    catalog
                        .get(&c.definition)
                        .map_err(|e| format!("{}: {e}", p.display()))?;
                }
                parts += bot.components.len();
                println!(
                    "OK {}: {} v{}, {} parts, {} controllers, {} bindings",
                    p.display(),
                    bot.name,
                    bot.source_version,
                    bot.components.len(),
                    bot.controllers.len(),
                    bot.bindings.len()
                );
            }
            Err(e) => {
                failed += 1;
                eprintln!("FAIL {}: {e}", p.display());
            }
        }
    }
    println!(
        "{} bot files, {parts} components, {failed} failed",
        files.len()
    );
    if failed > 0 {
        return Err("BOT audit failed".into());
    }
    Ok(())
}
