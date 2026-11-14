//! Original Gooey Interface Builder widget trees, including wrapper widgets.
//! Source fields are retained; layout and interaction belong to the runtime.
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Widget {
    pub kind: String,
    pub fields: BTreeMap<String, Vec<String>>,
    pub wrapped: Option<Box<Widget>>,
    pub children: Vec<Widget>,
}
impl Widget {
    pub fn value(&self, key: &str) -> &str {
        self.fields
            .get(key)
            .and_then(|v| v.last())
            .map_or("", String::as_str)
    }
    pub fn name(&self) -> &str {
        self.value("Name")
    }
    pub fn numbers(&self, key: &str) -> Vec<f32> {
        self.value(key)
            .split_whitespace()
            .filter_map(|v| v.parse::<f32>().ok())
            .collect()
    }
    pub fn flag(&self, key: &str) -> bool {
        self.value(key) == "true"
    }
    pub fn find(&self, name: &str) -> Option<&Widget> {
        if self.name() == name {
            return Some(self);
        }
        self.wrapped
            .as_deref()
            .and_then(|w| w.find(name))
            .or_else(|| self.children.iter().find_map(|w| w.find(name)))
    }
    pub fn visit(&self, f: &mut impl FnMut(&Widget)) {
        f(self);
        if let Some(w) = &self.wrapped {
            w.visit(f);
        }
        for child in &self.children {
            child.visit(f);
        }
    }
}

struct Reader<'a> {
    lines: Vec<&'a str>,
    pos: usize,
    count: usize,
}
impl Reader<'_> {
    fn line(&mut self) -> Result<&str, String> {
        let line = *self.lines.get(self.pos).ok_or("Unexpected end of GIB")?;
        self.pos += 1;
        Ok(line)
    }
    fn widget(&mut self, depth: usize) -> Result<Widget, String> {
        self.count += 1;
        if depth > 128 || self.count > 50_000 {
            return Err("GIB widget nesting/count limit".into());
        }
        let mut widget = Widget {
            kind: "Container".into(),
            fields: BTreeMap::new(),
            wrapped: None,
            children: vec![],
        };
        if self.lines.get(self.pos).is_some_and(|s| s.starts_with('[')) {
            widget.kind = self.line()?.trim_matches(['[', ']']).into();
        }
        loop {
            let next = self
                .lines
                .get(self.pos)
                .ok_or("Missing GIB common fields")?;
            if *next == "Name" || next.starts_with("Name ") {
                break;
            }
            let line = self.line()?.to_string();
            let (key, value) = line.split_once(' ').unwrap_or((&line, ""));
            widget
                .fields
                .entry(key.into())
                .or_default()
                .push(value.into());
            if line == "Widget true" || line == "Composition true" {
                widget.wrapped = Some(Box::new(self.widget(depth + 1)?));
                break;
            }
        }
        loop {
            let line = self.line()?.to_string();
            let (key, value) = line.split_once(' ').unwrap_or((&line, ""));
            widget
                .fields
                .entry(key.into())
                .or_default()
                .push(value.into());
            if key == "NumChildren" {
                let count: usize = value.parse().map_err(|_| "Invalid GIB child count")?;
                if count > 50_000 || count > self.lines.len().saturating_sub(self.pos) {
                    return Err("Oversized GIB child count".into());
                }
                for _ in 0..count {
                    widget.children.push(self.widget(depth + 1)?);
                }
                break;
            }
        }
        Ok(widget)
    }
}
pub fn decode(bytes: &[u8]) -> Result<Widget, String> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("GIB exceeds 16 MiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid GIB text")?;
    let mut reader = Reader {
        lines: text.lines().map(str::trim_end).collect(),
        pos: 1,
        count: 0,
    };
    if !reader.lines.first().is_some_and(|s| s.starts_with("T 1.")) {
        return Err("Unsupported GIB signature".into());
    }
    let widget = reader.widget(0)?;
    if reader.lines[reader.pos..].iter().any(|s| !s.is_empty()) {
        return Err(format!("Unconsumed GIB at line {}", reader.pos + 1));
    }
    Ok(widget)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapper_and_child_coordinates_remain_separate() {
        let bytes=b"T 1.09\n[Border]\nWidth 1\nWidget true\n[Button]\nName click\nPosition 10 0 20 0\nNumChildren 1\n[Image]\n_Filename botlab\\x.tga\nName picture\nNumChildren 0\nName border\nPosition 9 0 19 0\nNumChildren 0\n";
        let root = decode(bytes).unwrap();
        assert_eq!(root.kind, "Border");
        assert_eq!(
            root.find("click").unwrap().numbers("Position"),
            [10.0, 0.0, 20.0, 0.0]
        );
        assert_eq!(
            root.find("picture").unwrap().value("_Filename"),
            "botlab\\x.tga"
        );
        assert!(decode(&bytes[..bytes.len() - 5]).is_err());
    }
}
