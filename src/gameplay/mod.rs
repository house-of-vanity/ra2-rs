//! Shared articulated robot runtime used by the workshop and Exhibition.
pub(crate) mod camera;
pub(crate) mod damage;
pub(crate) mod drive;
pub(crate) mod mass;
pub(crate) mod numbers;
pub(crate) mod spawn;
pub(crate) mod surface;
pub(crate) mod tyres;
use arena_core::mechanism::Command;
use arena_core::{mechanism::*, robot::RobotDefinition};
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
#[derive(Component)]
pub(crate) struct RobotBody;
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct RobotOwner(pub u32);
pub(crate) struct RobotInstance {
    pub id: u32,
    pub tuning: crate::config::GarageConfig,
    pub context: Entity,
    pub rig: Option<Rig>,
    pub document: Option<RobotDefinition>,
    pub bodies: Vec<Entity>,
    pub controls: Controls,
    pub commands: Vec<Command>,
    pub states: Vec<ActuatorState>,
    pub active: bool,
    pub running: bool,
    pub damage: damage::State,
    /// Ground frame in the scene; spawning offsets the assembly bounds above it.
    pub placement: Option<Transform>,
    pub group: Group,
    pub layer: usize,
}
impl RobotInstance {
    pub fn new(id: u32, context: Entity, tuning: crate::config::GarageConfig) -> Self {
        Self {
            id,
            context,
            tuning,
            rig: None,
            document: None,
            bodies: vec![],
            controls: Controls::default(),
            commands: vec![],
            states: vec![],
            active: false,
            running: false,
            damage: damage::State::default(),
            placement: None,
            group: Group::GROUP_2,
            layer: 9,
        }
    }
    pub fn clear(&mut self, commands: &mut Commands) {
        for e in self.damage.fragments.drain(..).chain(self.bodies.drain(..)) {
            commands.entity(e).despawn();
        }
        self.rig = None;
        self.document = None;
        self.active = false;
        self.running = false;
    }
}
/// Composition adapter; math and robot state do not depend on a workshop page.
pub(crate) fn instances_mut<'a>(
    garage: &'a mut crate::garage::Garage,
    battle: Option<&'a mut crate::battle::Session>,
) -> impl Iterator<Item = &'a mut RobotInstance> {
    std::iter::once(&mut garage.instance)
        .chain(battle.into_iter().flat_map(|s| s.robots.iter_mut()))
}
pub(crate) fn instances<'a>(
    garage: &'a crate::garage::Garage,
    battle: Option<&'a crate::battle::Session>,
) -> impl Iterator<Item = &'a RobotInstance> {
    std::iter::once(&garage.instance).chain(battle.into_iter().flat_map(|s| s.robots.iter()))
}
pub(crate) struct GameplayPlugin;
impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_seconds(crate::STEP))
            .insert_resource(TimestepMode::Fixed {
                dt: crate::STEP as f32,
                substeps: 1,
            })
            .add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule())
            .add_systems(
                FixedUpdate,
                (drive::motors, damage::capture, tyres::contacts)
                    .chain()
                    .before(PhysicsSet::SyncBackend),
            )
            .add_systems(FixedUpdate, damage::contacts.after(PhysicsSet::Writeback))
            .add_systems(
                Update,
                numbers::project
                    .after(crate::legacy_ui::Paint)
                    .run_if(resource_exists::<crate::legacy_ui::Output>),
            );
    }
}
