//! The original four-line records in RA2.cfz / Armor_Definitions.txt.
#[derive(Clone, Debug)]
pub struct ArmorDefinition {
    pub name: String,
    pub description: String,
    pub image: String,
    pub id: u32,
    pub density: f32,
    pub flags: [f32; 2],
    pub hardness: f32,
}
pub fn decode(bytes: &[u8]) -> Result<Vec<ArmorDefinition>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if lines.len() % 4 != 0 {
        return Err("Incomplete armor definition".into());
    }
    let mut result: Vec<ArmorDefinition> = vec![];
    for [name, description, image, numbers] in lines.as_chunks::<4>().0 {
        let values = numbers
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if values.len() != 5
            || values.iter().any(|f| !f.is_finite())
            || values[0] < 0.0
            || values[0].fract() != 0.0
            || values[1] <= 0.0
            || values[4] <= 0.0
        {
            return Err("Invalid armor properties".into());
        }
        let id = values[0] as u32;
        if result.iter().any(|a| a.id == id) {
            return Err("Duplicate armor ID".into());
        }
        result.push(ArmorDefinition {
            name: (*name).into(),
            description: (*description).into(),
            image: (*image).into(),
            id,
            density: values[1],
            flags: [values[2], values[3]],
            hardness: values[4],
        });
    }
    if result.is_empty() {
        return Err("Armor definitions are empty".into());
    }
    Ok(result)
}
