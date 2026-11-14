//! CFL3/CFZ component definitions. Archive parsing never writes extracted files.
use crate::archive::CfzArchive;
use std::collections::BTreeMap;

pub fn normalize(path: &str) -> String {
    path.replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase()
}

#[derive(Clone, Debug)]
pub struct ComponentDefinition {
    pub fields: BTreeMap<String, String>,
}
impl ComponentDefinition {
    pub fn parse(text: &str) -> Self {
        Self {
            fields: text
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if line.starts_with('#') || line.starts_with(';') || line.starts_with("//") {
                        return None;
                    }
                    let (key, value) = line.split_once('=')?;
                    Some((key.trim().to_ascii_lowercase(), value.trim().to_string()))
                })
                .collect(),
        }
    }
    pub fn value(&self, key: &str) -> &str {
        self.fields.get(key).map_or("", String::as_str)
    }
    pub fn model_path(&self) -> Result<String, String> {
        if self.value("dir").is_empty() || self.value("model").is_empty() {
            return Err("Component has no dir/model".into());
        }
        Ok(format!("Components/{}/{}", self.value("dir"), self.value("model")).replace('\\', "/"))
    }
}

#[derive(Default)]
pub struct ComponentCatalog {
    pub definitions: BTreeMap<String, ComponentDefinition>,
}
impl ComponentCatalog {
    pub fn from_cfz(bytes: &[u8]) -> Result<Self, String> {
        let archive = CfzArchive::decode(bytes)?;
        let definitions = archive
            .files
            .into_iter()
            .map(|(name, bytes)| {
                (
                    name,
                    ComponentDefinition::parse(&String::from_utf8_lossy(&bytes)),
                )
            })
            .collect();
        Ok(Self { definitions })
    }

    pub fn get(&self, path: &str) -> Result<&ComponentDefinition, String> {
        self.definitions
            .get(&normalize(path))
            .ok_or_else(|| format!("Component definition not found: {path}"))
    }
}
