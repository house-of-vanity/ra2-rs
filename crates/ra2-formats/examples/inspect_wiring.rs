fn main() {
    let root = std::env::args().nth(1).unwrap();
    fn visit(p: &std::path::Path) {
        if p.is_dir() {
            for e in std::fs::read_dir(p).unwrap() {
                visit(&e.unwrap().path());
            }
        } else if p.extension().is_some_and(|s| s.eq_ignore_ascii_case("bot")) {
            let r = ra2_formats::bot::decode(&std::fs::read(p).unwrap()).unwrap();
            println!("{} controls={:?}", r.name, r.controllers);
            for b in &r.bindings {
                println!(
                    "{} -> {} {:?}",
                    r.components[b.component].class,
                    b.action,
                    (b.controller, b.key)
                );
            }
        }
    }
    visit(std::path::Path::new(&root));
}
