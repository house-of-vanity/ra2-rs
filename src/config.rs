//! Demo/game rules are configuration, not assumptions inside importers or adapters.
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub arena: ArenaConfig,
    pub robot: RobotConfig,
    pub rendering: RenderingConfig,
    pub showcase: Vec<ShowcaseConfig>,
    #[serde(default)]
    pub garage: GarageConfig,
}
/// Conversion from the original motor's gameplay force units to this solver's SI units.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GarageConfig {
    pub motor_force_scale: f32,
    pub motor_damping: f32,
    pub servo_stiffness: f32,
    pub servo_damping: f32,
    pub strike_speed_scale: f32,
    pub strike_force_scale: f32,
    pub smooth_wheel_contacts: bool,
    /// Native impact strength -> HP in this solver; independent of mesh density.
    pub impact_damage_scale: f32,
    /// Weight/radius based drive torque budget, in m/s². Does not limit RPM.
    pub wheel_drive_acceleration: f32,
    /// Torque response per rad/s of wheel-speed error, independent of inertia.
    pub wheel_motor_damping: f32,
}
impl Default for GarageConfig {
    fn default() -> Self {
        Self {
            motor_force_scale: 40.0,
            motor_damping: 20.0,
            servo_stiffness: 400.0,
            servo_damping: 35.0,
            strike_speed_scale: 1.35,
            strike_force_scale: 1.6,
            smooth_wheel_contacts: true,
            impact_damage_scale: 0.25,
            wheel_drive_acceleration: 8.0,
            wheel_motor_damping: 100.0,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaConfig {
    pub title: String,
    pub model: String,
    pub floor_object: String,
    pub collision_object: String,
    pub robot_spawn: [f32; 3],
    pub friction: f32,
    pub restitution: f32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotConfig {
    pub chassis_size: [f32; 3],
    pub chassis_mass: f32,
    pub wheel_mass: f32,
    pub wheel_radius: f32,
    pub wheel_width: f32,
    pub wheel_track: f32,
    pub wheel_base: f32,
    pub axle_height: f32,
    pub motor_speed: f32,
    pub turn_speed: f32,
    pub motor_damping: f32,
    pub motor_force: f32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderingConfig {
    pub roughness: f32,
    pub opacity_map_alpha: f32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowcaseConfig {
    pub label: String,
    pub model: String,
    pub position: [f32; 3],
    pub size: f32,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = if path == Path::new("@embedded") {
            include_str!("../config/demo.ron").to_owned()
        } else {
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?
        };
        let config: Self = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if config.version != 1 {
            return Err(format!(
                "Unsupported demo configuration version {}",
                config.version
            ));
        }
        let r = &config.robot;
        let positive = [
            r.chassis_mass,
            r.wheel_mass,
            r.wheel_radius,
            r.wheel_width,
            r.wheel_track,
            r.wheel_base,
            r.motor_speed,
            r.turn_speed,
            r.motor_damping,
            r.motor_force,
            config.garage.motor_force_scale,
            config.garage.motor_damping,
            config.garage.servo_stiffness,
            config.garage.servo_damping,
            config.garage.strike_speed_scale,
            config.garage.strike_force_scale,
            config.garage.impact_damage_scale,
            config.garage.wheel_drive_acceleration,
            config.garage.wheel_motor_damping,
        ];
        if positive
            .iter()
            .chain(r.chassis_size.iter())
            .any(|v| !v.is_finite() || *v <= 0.0)
            || !r.axle_height.is_finite()
            || !config.arena.robot_spawn.iter().all(|v| v.is_finite())
            || !config.arena.friction.is_finite()
            || config.arena.friction < 0.0
            || !(0.0..=1.0).contains(&config.arena.restitution)
            || !(0.0..=1.0).contains(&config.rendering.roughness)
            || !(0.0..=1.0).contains(&config.rendering.opacity_map_alpha)
            || config.showcase.iter().any(|s| {
                !s.size.is_finite() || s.size <= 0.0 || s.position.iter().any(|v| !v.is_finite())
            })
        {
            return Err("Invalid scene/robot parameters in configuration".into());
        }
        Ok(config)
    }
}
