//! Small mutations of original widget trees, shared by single-player and multiplayer menus.
use super::assets::LegacyAssets;
use ra2_formats::gib::Widget;
pub(crate) fn rename(w: &mut Widget, prefix: &str) {
    if let Some(names) = w.fields.get_mut("Name") {
        for n in names.iter_mut().filter(|n| !n.is_empty()) {
            *n = format!("{prefix}{n}");
        }
    }
    if let Some(c) = &mut w.wrapped {
        rename(c, prefix);
    }
    for c in &mut w.children {
        rename(c, prefix);
    }
}
pub(crate) fn image(w: &mut Widget, name: &str, path: &str) {
    crate::editor::panels::patch(w, name, "_Filename", path);
}
pub(crate) fn visible(w: &mut Widget, name: &str, value: bool) {
    crate::editor::panels::patch(w, name, "Visible", if value { "true" } else { "false" });
}
pub(crate) fn widget(a: &LegacyAssets, path: &str) -> Option<Widget> {
    a.widget(path).ok().map(|w| (*w).clone())
}
