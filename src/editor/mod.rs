//! Robot workshop: source GUI, neutral editable document and native file adapter.
pub(crate) mod catalog;
pub(crate) mod controls;
mod heading;
mod painting;
pub(crate) mod panels;
mod placement;
mod preview;
mod smoke;
pub(crate) mod viewport;
mod wiring;
use crate::{
    Options,
    frontend::{Navigation, Page},
    legacy_ui::{self, Cell, Lines, assets::LegacyAssets},
};
use arena_core::editor::Workshop;
use bevy::prelude::*;
use catalog::Catalog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Overview,
    Chassis,
    Base,
    Top,
    Armor,
    Components,
    Wiring,
    Paint,
    Test,
}
#[derive(Clone, Debug)]
pub enum Dialog {
    SavePrompt,
    Export,
    Import,
    Controller(u32),
    Connection,
    Message(String),
    PaintImport,
    PaintExport,
    Help,
}
#[derive(Resource)]
pub struct Editor {
    pub document: Workshop,
    pub team_slot: Option<(usize, usize)>,
    pub team_root: Option<std::path::PathBuf>,
    pub return_page: Page,
    pub exit_page: Page,
    pub mode: Mode,
    pub category: String,
    pub component: String,
    pub selected: Option<usize>,
    pub pending: Option<String>,
    pub selected_socket: u32,
    pub attach_angle: f32,
    pub attach_rotation_remainder: f32,
    pub dialog: Option<Dialog>,
    pub snap: bool,
    pub outline: Vec<Vec2>,
    pub top: Vec<Vec2>,
    pub closed: bool,
    pub height: f32,
    pub color: [u8; 4],
    pub hue_color: [u8; 4],
    pub luminance: f32,
    pub paint_effect: usize,
    pub selected_controller: Option<usize>,
    pub selected_control_slot: [i32; 2],
    pub source: Option<std::path::PathBuf>,
    pub status: String,
    pub ui_revision: u64,
    pub exit_after_save: bool,
    pub controller_kind: u32,
    pub key_positive: i32,
    pub key_negative: i32,
    pub key_native: [Option<Vec<i32>>; 2],
    pub capture_key: Option<usize>,
    pub active_point: Option<usize>,
    pub draw_style: usize,
    pub radius: f32,
    pub length: f32,
    pub width: f32,
    pub binding_actions: [usize; 2],
    pub snapshot_requested: bool,
    pub properties_tab: usize,
    pub preview_burst_end: bool,
    pub paint_texture: String,
    pub decal_scale: f32,
    pub paint_export_current: bool,
    pub attach_offset: f32,
    pub ghost_pick: Option<viewport::Pick>,
    pub ghost_pose: Option<Mat4>,
    pub ghost_model: Option<ra2_formats::assembly::ResolvedComponent>,
    pub ghost_valid: bool,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            document: Workshop::new(),
            team_slot: None,
            team_root: None,
            return_page: Page::Main,
            exit_page: Page::Main,
            mode: Mode::Overview,
            category: "power".into(),
            component: String::new(),
            selected: None,
            pending: None,
            selected_socket: 1,
            attach_angle: 0.0,
            attach_rotation_remainder: 0.0,
            dialog: None,
            snap: true,
            outline: vec![],
            top: vec![],
            closed: false,
            height: 0.35,
            color: [190, 190, 190, 255],
            hue_color: [190, 190, 190, 255],
            luminance: 0.5,
            paint_effect: 0,
            selected_controller: None,
            selected_control_slot: [0, 0],
            source: None,
            status: String::new(),
            ui_revision: 0,
            exit_after_save: false,
            controller_kind: 1,
            key_positive: 17,
            key_negative: 31,
            key_native: [None, None],
            capture_key: None,
            active_point: None,
            draw_style: 0,
            radius: 300.0,
            length: 600.0,
            width: 600.0,
            binding_actions: [0, 1],
            snapshot_requested: false,
            properties_tab: 0,
            preview_burst_end: false,
            paint_texture: String::new(),
            decal_scale: 0.3,
            paint_export_current: false,
            attach_offset: 0.0,
            ghost_pick: None,
            ghost_pose: None,
            ghost_model: None,
            ghost_valid: false,
        }
    }
}
#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Edit;
pub struct EditorPlugin;
impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editor>()
            .add_systems(
                Startup,
                (setup, viewport::setup)
                    .chain()
                    .after(crate::legacy_ui::assets::load),
            )
            .configure_sets(Update, (legacy_ui::Input, Edit, legacy_ui::Paint).chain())
            .add_systems(
                Update,
                (controls::input, viewport::update, sync)
                    .chain()
                    .in_set(Edit),
            )
            .add_systems(Update, smoke::run.after(Edit).before(legacy_ui::Paint));
    }
}
pub(crate) fn setup(
    mut commands: Commands,
    mut assets: ResMut<LegacyAssets>,
    options: Res<Options>,
    mut editor: ResMut<Editor>,
) {
    let catalog = Catalog::load(&options.game_dir, &mut assets).unwrap_or_else(|e| {
        warn!("Workshop catalog: {e}");
        Catalog::default()
    });
    editor.component = catalog
        .browser("power")
        .first()
        .map(|a| a.key.clone())
        .unwrap_or_default();
    if let Some(asset) = catalog.components.get(&editor.component) {
        editor.selected_socket = placement::default_socket(&asset.model);
    }
    if let Some(path) = &options.editor_file {
        let path = if path.is_file() {
            path.clone()
        } else {
            options.game_dir.join(path)
        };
        if let Err(e) = controls::load(&mut editor, &path) {
            editor.status = e;
        }
    }
    commands.insert_resource(catalog);
}
#[allow(
    clippy::too_many_arguments,
    reason = "Workshop document, GUI, garage and rendering state"
)]
fn sync(
    editor: Res<Editor>,
    mut assets: ResMut<LegacyAssets>,
    view: Res<viewport::View>,
    catalog: Res<Catalog>,
    garage: Res<crate::garage::Garage>,
    nav: Res<Navigation>,
    mut ui: ResMut<legacy_ui::State>,
    mut last: Local<(u64, u64, Option<Mode>, bool)>,
) {
    let visible =
        nav.page == Page::Workshop || (nav.page == Page::Settings && nav.back == Page::Workshop);
    if !visible {
        last.3 = false;
        return;
    }
    let stamp = (
        editor.document.revision,
        editor.ui_revision,
        Some(editor.mode),
        true,
    );
    if *last == stamp {
        return;
    }
    *last = stamp;
    ui.values.retain(|key, _| {
        matches!(
            key.as_str(),
            "file name" | "control name" | "import path" | "export name textbox"
        )
    });
    ui.root = assets.widget("botlab/botlabwin.gib").ok();
    ui.slots.clear();
    ui.dialogs.clear();
    ui.cells.clear();
    ui.lines.clear();
    ui.floating_text.clear();
    ui.checked.clear();
    ui.viewports.clear();
    ui.disabled.clear();
    if editor.document.robot.chassis.model.is_none() {
        for name in [
            "components button",
            "wiring button",
            "paintshop button",
            "test robot button",
            "armor button",
        ] {
            ui.disabled.insert(name.into());
        }
    }
    if !editor
        .document
        .robot
        .components
        .iter()
        .any(|p| p.class == "ControlBoard")
    {
        ui.disabled.insert("wiring button".into());
    }

    ui.attachments.clear();
    let panel = match editor.mode {
        Mode::Overview => "overview",
        Mode::Chassis => "chassis",
        Mode::Base => "chassis_structure1",
        Mode::Top => "chassis_structure2",
        Mode::Armor => "armor",
        Mode::Components => "components",
        Mode::Wiring => "wiring",
        Mode::Paint => "appearance",
        Mode::Test => "testarena",
    };
    if let Ok(panel) = assets.widget(&format!("botlab/{panel}.gib")) {
        ui.slots.insert("content slot".into(), panel);
    }
    for (name, selected) in [
        ("overview button", editor.mode == Mode::Overview),
        (
            "chassis button",
            matches!(
                editor.mode,
                Mode::Chassis | Mode::Base | Mode::Top | Mode::Armor
            ),
        ),
        ("components button", editor.mode == Mode::Components),
        ("wiring button", editor.mode == Mode::Wiring),
        ("paintshop button", editor.mode == Mode::Paint),
        ("test robot button", editor.mode == Mode::Test),
    ] {
        ui.checked.insert(name.into(), selected);
    }
    ui.values.insert(
        "bot name textbox".into(),
        editor.document.robot.name.clone(),
    );
    ui.values.insert(
        "weightclass".into(),
        match editor.document.robot.weight_class {
            0 => "Lightweight",
            1 => "Middleweight",
            _ => "Heavyweight",
        }
        .into(),
    );
    ui.values.insert(
        "heading slider".into(),
        heading::slider_value(editor.document.robot.heading).to_string(),
    );
    if matches!(editor.mode, Mode::Base | Mode::Top) {
        if let Ok(buttons) = assets.widget("botlab/ch_stepbuttons.gib") {
            ui.attachments.push(("content slot".into(), buttons));
        }
        ui.checked
            .insert("step 1 button".into(), editor.mode == Mode::Base);
        ui.checked
            .insert("step 2 button".into(), editor.mode == Mode::Top);
        ui.checked.insert("snap checkbox".into(), editor.snap);
        ui.checked
            .insert("freeform checkbox".into(), editor.draw_style == 0);
        ui.checked
            .insert("circle checkbox".into(), editor.draw_style == 1);
        ui.checked
            .insert("rectangle checkbox".into(), editor.draw_style == 2);
        ui.lines.push(Lines {
            region: "Editing Area".into(),
            points: (0..=20)
                .flat_map(|y| (0..=20).map(move |x| Vec2::new(x as f32, y as f32) / 20.))
                .collect(),
            color: [0.55, 0.65, 0.7, 0.8],
            width: 0.,
            dots: true,
            dot_size: 2.,
            dot_image: None,
        });
        let points = if editor.mode == Mode::Top {
            &editor.top
        } else {
            &editor.outline
        };
        let mut points: Vec<_> = points.iter().map(|p| *p / 1000.0).collect();
        if editor.closed && !points.is_empty() {
            points.push(points[0]);
        }
        ui.lines.push(Lines {
            region: "Editing Area".into(),
            points,
            color: [1.0, 1.0, 0.0, 1.0],
            width: 1.0,
            dots: true,
            dot_size: 5.,
            dot_image: None,
        });
        if let Some(index) = editor.active_point
            && let Some(point) = if editor.mode == Mode::Top {
                editor.top.get(index)
            } else {
                editor.outline.get(index)
            }
        {
            ui.lines.push(Lines {
                region: "Editing Area".into(),
                points: vec![*point / 1000.],
                color: [1.; 4],
                width: 0.,
                dots: true,
                dot_size: 16.,
                dot_image: Some("point selection.tga".into()),
            });
        }
    }
    if editor.mode == Mode::Components {
        for (name, cat) in [
            ("power button", "power"),
            ("extenders button", "extenders"),
            ("mechanics button", "mechanics"),
            ("mobility button", "mobility"),
            ("weapons button", "weapons"),
            ("extras button", "extras"),
        ] {
            ui.checked.insert(name.into(), editor.category == cat);
        }
        ui.cells.insert(
            "components grid".into(),
            catalog
                .browser(&editor.category)
                .iter()
                .map(|a| Cell {
                    key: format!("component:{}", a.key),
                    image: a.preview.clone(),
                    selected: editor.component == a.key,
                    ..default()
                })
                .collect(),
        );
        if let Ok(panel) = assets.widget("botlab/comp_preview.gib") {
            ui.slots.insert("component content group".into(), panel);
        }
        if let Some(a) = catalog.components.get(&editor.component) {
            ui.values
                .insert("component name textbox".into(), a.name().into());
            ui.values.insert(
                "component description".into(),
                a.definition.value("description").into(),
            );
            ui.values.insert(
                "component weight textbox".into(),
                format!(
                    "{:.0}",
                    a.model.all_bodies().iter().map(|b| b.mass).sum::<f32>()
                ),
            );
        }
    }
    for name in [
        "3d placeholder",
        "3d component placeholder",
        "3d wiring placeholder",
        "3d appearance placeholder",
        "3d preview placeholder",
        "3d view placeholder",
        "component 3d preview",
        "test arena placeholder",
    ] {
        ui.viewports.insert(name.into());
    }
    if editor.mode == Mode::Wiring {
        ui.cells.insert(
            "control grid".into(),
            (0..12)
                .map(|i| {
                    let position = [i % 4, i / 4];
                    let controller = editor
                        .document
                        .robot
                        .controllers
                        .iter()
                        .find(|c| c.position == position);
                    Cell {
                        key: format!("control:{i}"),
                        label: controller
                            .map(|c| c.name.trim_start_matches("Name: ").to_string())
                            .unwrap_or_default(),
                        image: controller
                            .map(|c| {
                                format!(
                                    "botlab/ctrl_{}.bmp",
                                    match c.kind {
                                        1 => "analog",
                                        2 => "switch",
                                        _ => "button",
                                    }
                                )
                            })
                            .unwrap_or_default(),
                        selected: editor.selected_control_slot == position,
                        ..default()
                    }
                })
                .collect(),
        );
    }
    panels::sync(&editor, &catalog, &mut assets, &mut ui);
    if matches!(editor.mode, Mode::Base | Mode::Top) {
        ui.interactive.insert("Editing Area".into());
    }
    if editor.mode == Mode::Test {
        crate::garage::bind_ui(&garage, &mut ui, &assets);
    }
    if let Some(assembly) = &view.assembly {
        ui.values
            .insert("weight".into(), format!("{:.1}", assembly.mass));
    }
    if let Some(dialog) = &editor.dialog {
        let file = match dialog {
            Dialog::SavePrompt => "botlab/save_dialog",
            Dialog::Export => "botlab/exportbot",
            Dialog::Import => "botlab/importbot",
            Dialog::Controller(1) => "botlab/ctrl_analog_props",
            Dialog::Controller(2) => "botlab/ctrl_switch_props",
            Dialog::Controller(_) => "botlab/ctrl_button_props",
            Dialog::Connection => match editor
                .selected_controller
                .and_then(|i| editor.document.robot.controllers.get(i))
                .map(|c| c.kind)
            {
                Some(2) => "botlab/wiring_connection_switch",
                Some(4) => "botlab/wiring_connection_button",
                _ => "botlab/wiring_connection_analog",
            },
            Dialog::Message(_) => "common/messagebox",
            Dialog::PaintImport => "botlab/import_texture",
            Dialog::PaintExport => "botlab/export_texture",
            Dialog::Help => "help/helpframe",
        };
        if let Ok(panel) = assets.widget(&format!("{file}.gib")) {
            ui.dialogs.push(panel);
        }
    }
    ui.revision += 1;
}
