//! Editable robot state and construction operations. No filesystem, UI or physics backend.
pub mod chassis;
pub mod paint;
/// Original 1.3 executable's classification thresholds (0x004df080).
pub fn weight_class(mass: f32) -> u32 {
    if mass < 249.0 {
        0
    } else if mass < 399.0 {
        1
    } else {
        2
    }
}

use crate::robot::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Revision {
    pub label: String,
    pub robot: RobotDefinition,
}
pub struct Workshop {
    pub robot: RobotDefinition,
    pub revision: u64,
    pub dirty: bool,
    undo: Vec<Revision>,
    redo: Vec<Revision>,
}
impl Default for Workshop {
    fn default() -> Self {
        Self::new()
    }
}
impl Workshop {
    pub fn new() -> Self {
        let robot = RobotDefinition {
            name: "New Robot".into(),
            source_version: "1.12".into(),
            weight_class: 0,
            components: vec![RobotComponent {
                parent: None,
                parent_mount: 0,
                own_mount: 0,
                mounting: MountPlacement {
                    twist: std::f32::consts::PI,
                    ..Default::default()
                },
                class: "Chassis".into(),
                definition: String::new(),
                settings: BTreeMap::new(),
            }],
            chassis: Chassis {
                bottom: vec![],
                top: vec![],
                height: 0.35,
                model: None,
                texture: None,
                armour: 0,
                surface_density: 8.0,
                hardness: 80.0,
                buckling_points: vec![],
                face_count: 0,
                paint_layout: ChassisPaintLayout::default(),
                material_parameters: [0.2, 0.2, 0.2, 0.85, 20.0],
                armour_flags: [0.0; 2],
            },
            snapshot: None,
            damaged_model: None,
            heading: 0.0,
            controllers: vec![],
            bindings: vec![],
            default_paint: 1,
        };
        Self {
            robot,
            revision: 0,
            dirty: false,
            undo: vec![],
            redo: vec![],
        }
    }
    pub fn load(&mut self, robot: RobotDefinition) {
        self.robot = robot;
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
        self.dirty = false;
    }
    pub fn edit(
        &mut self,
        label: &str,
        change: impl FnOnce(&mut RobotDefinition) -> Result<(), String>,
    ) -> Result<(), String> {
        let before = self.robot.clone();
        if let Err(e) = change(&mut self.robot) {
            self.robot = before;
            return Err(e);
        }
        self.undo.push(Revision {
            label: label.into(),
            robot: before,
        });
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision += 1;
        self.dirty = true;
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Revision {
            label: before.label,
            robot: self.robot.clone(),
        });
        self.robot = before.robot;
        self.revision += 1;
        self.dirty = true;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Revision {
            label: after.label,
            robot: self.robot.clone(),
        });
        self.robot = after.robot;
        self.revision += 1;
        self.dirty = true;
        true
    }
    pub fn remove_component(&mut self, index: usize) -> Result<(), String> {
        if index == 0 || index >= self.robot.components.len() {
            return Err("Select a non-chassis component".into());
        }
        self.edit("Remove component", |robot| {
            let mut removed = vec![false; robot.components.len()];
            removed[index] = true;
            loop {
                let mut changed = false;
                for i in 1..removed.len() {
                    if !removed[i] && robot.components[i].parent.is_some_and(|p| removed[p]) {
                        removed[i] = true;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            let mut map = vec![0; removed.len()];
            let mut next = 0;
            for i in 0..removed.len() {
                if !removed[i] {
                    map[i] = next;
                    next += 1;
                }
            }
            robot.components = robot
                .components
                .iter()
                .enumerate()
                .filter(|(i, _)| !removed[*i])
                .map(|(_, c)| {
                    let mut c = c.clone();
                    c.parent = c.parent.map(|p| map[p]);
                    c
                })
                .collect();
            robot.bindings.retain(|b| !removed[b.component]);
            for b in &mut robot.bindings {
                b.component = map[b.component];
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_weight_class_boundaries_are_strict() {
        assert_eq!(weight_class(248.999), 0);
        assert_eq!(weight_class(249.0), 1);
        assert_eq!(weight_class(398.999), 1);
        assert_eq!(weight_class(399.0), 2);
    }
    #[test]
    fn failed_edits_are_atomic_and_history_restores_geometry() {
        let mut doc = Workshop::new();
        assert!(
            doc.edit("bad", |r| {
                r.name = "bad".into();
                Err("fail".into())
            })
            .is_err()
        );
        assert_eq!(doc.robot.name, "New Robot");
        assert!(!doc.undo());
        doc.edit("rename", |r| {
            r.name = "Fixture".into();
            Ok(())
        })
        .unwrap();
        assert!(doc.undo());
        assert_eq!(doc.robot.name, "New Robot");
        assert!(doc.redo());
        assert_eq!(doc.robot.name, "Fixture");
    }
}
