//! Single-player Exhibition setup. Match execution lives in the shared battle runtime.
pub(crate) mod check;
mod ui;
use crate::battle::*;
use crate::{
    frontend::{Navigation, Page},
    legacy_ui,
};
use arena_core::match_rules::Mode;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
pub(crate) struct ExhibitionPlugin;
impl Plugin for ExhibitionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (crate::battle::refresh, ui::input, ui::bind)
                .chain()
                .after(crate::legacy_ui::Input)
                .after(crate::battle::lifecycle)
                .before(crate::legacy_ui::Paint),
        );
    }
}

use crate::battle::arena;
