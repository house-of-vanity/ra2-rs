//! Native Teams/Teams.txt. Unknown league/save fields are retained byte for byte.
#[derive(Clone, Debug)]
pub struct Team {
    pub lines: Vec<Vec<u8>>,
}
impl Team {
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.lines[0]).into_owned()
    }
    pub fn motto(&self) -> String {
        String::from_utf8_lossy(&self.lines[1]).into_owned()
    }
    pub fn logo(&self) -> String {
        String::from_utf8_lossy(&self.lines[2]).replace('\\', "/")
    }
    pub fn robots(&self) -> Vec<usize> {
        String::from_utf8_lossy(&self.lines[3])
            .split_whitespace()
            .skip(1)
            .filter_map(|v| v.parse().ok())
            .collect()
    }
    pub fn set_identity(&mut self, name: &str, motto: &str, logo: &str) -> Result<(), String> {
        if name.trim().is_empty()
            || [name, motto, logo]
                .iter()
                .any(|s| s.contains(['\r', '\n', '\0']))
        {
            return Err("Enter a team name; team fields must not contain line breaks".into());
        }
        self.lines[0] = name.trim().as_bytes().to_vec();
        self.lines[1] = motto.as_bytes().to_vec();
        self.lines[2] = logo.replace('/', "\\").into_bytes();
        Ok(())
    }
    pub fn set_robot(&mut self, slot: usize, present: bool) -> Result<(), String> {
        if slot >= 6 {
            return Err("A team has six robot slots".into());
        }
        let mut robots = self.robots();
        robots.retain(|&i| i != slot);
        if present {
            robots.push(slot);
        }
        robots.sort_unstable();
        self.lines[3] = format!(
            "Robots:{}",
            robots.iter().map(|i| format!(" {i}")).collect::<String>()
        )
        .into_bytes();
        Ok(())
    }
    pub fn empty() -> Self {
        let text = "\n\n\nRobots:\nRobot In Event: -1\n0\n16\n16\n0\n0\n0\n0\n0\n10000\ntrue\n0\nfalse\n0\n-1\n0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 \n0\n0\n0";
        Self {
            lines: text.split('\n').map(|l| l.as_bytes().to_vec()).collect(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Teams {
    pub active: Vec<usize>,
    pub records: Vec<Team>,
    header: Vec<u8>,
    original_active: Vec<usize>,
    crlf: bool,
    terminated: bool,
}
impl Default for Teams {
    fn default() -> Self {
        Self {
            active: vec![],
            records: (0..16).map(|_| Team::empty()).collect(),
            header: vec![],
            original_active: vec![],
            crlf: true,
            terminated: true,
        }
    }
}
impl Teams {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut lines: Vec<_> = bytes
            .split(|&b| b == b'\n')
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
            .collect();
        let terminated = bytes.ends_with(b"\n");
        if terminated {
            lines.pop();
        }
        let header = lines.first().ok_or("Empty Teams.txt")?.clone();
        let active: Vec<usize> = std::str::from_utf8(&header)
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .map(|s| {
                s.parse()
                    .map_err(|_| "Invalid native team index".to_string())
            })
            .collect::<Result<_, _>>()?;
        let starts: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                l.starts_with(b"Robots:")
                    && lines
                        .get(i + 1)
                        .is_some_and(|next| next.starts_with(b"Robot In Event:"))
            })
            .map(|(i, _)| i.checked_sub(3).ok_or("Truncated team record".to_string()))
            .collect::<Result<_, _>>()?;
        if starts.first() != Some(&1) || starts.len() != 16 {
            return Err("Expected sixteen native team records".into());
        }
        if active.iter().any(|&i| i >= starts.len())
            || active
                .iter()
                .enumerate()
                .any(|(i, v)| active[..i].contains(v))
        {
            return Err("Invalid active team list".into());
        }
        let records = starts
            .iter()
            .enumerate()
            .map(|(i, &begin)| {
                let end = starts.get(i + 1).copied().unwrap_or(lines.len());
                if end - begin < 23 || !lines[begin + 4].starts_with(b"Robot In Event:") {
                    return Err("Truncated native team state".into());
                }
                let record = Team {
                    lines: lines[begin..end].to_vec(),
                };
                if record.robots().iter().any(|&slot| slot >= 6) {
                    return Err("Invalid robot slot".into());
                }
                Ok(record)
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            original_active: active.clone(),
            active,
            records,
            header,
            crlf: bytes.windows(2).any(|b| b == b"\r\n"),
            terminated,
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        let header = if self.active == self.original_active {
            self.header.clone()
        } else {
            self.active
                .iter()
                .map(|i| format!("{i} "))
                .collect::<String>()
                .into_bytes()
        };
        let lines: Vec<_> = std::iter::once(header)
            .chain(self.records.iter().flat_map(|r| r.lines.iter().cloned()))
            .collect();
        let mut bytes = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            bytes.extend(line);
            if i + 1 < lines.len() || self.terminated {
                bytes.extend(if self.crlf { b"\r\n".as_slice() } else { b"\n" });
            }
        }
        bytes
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_preserve_unknown_native_season_data_and_inactive_slots() {
        let mut source = Teams::default();
        source.active = vec![0, 5];
        source.records[0]
            .set_identity("One", "Motto", "Teams/rat.bmp")
            .unwrap();
        source.records[5]
            .set_identity("Five", "", "Teams/Atom.bmp")
            .unwrap();
        source.records[0].lines.insert(18, vec![0xff, b' ', b'1']);
        let bytes = source.encode();
        let mut loaded = Teams::decode(&bytes).unwrap();
        assert_eq!(loaded.encode(), bytes);
        let untouched = loaded.records[5].lines.clone();
        let opaque = loaded.records[0].lines[4..].to_vec();
        loaded.records[0]
            .set_identity("Renamed", "New motto", "Teams/Fire.bmp")
            .unwrap();
        loaded.records[0].set_robot(2, true).unwrap();
        loaded.records[0].set_robot(5, true).unwrap();
        loaded.records[0].set_robot(2, false).unwrap();
        let reloaded = Teams::decode(&loaded.encode()).unwrap();
        assert_eq!(reloaded.records[0].robots(), [5]);
        assert_eq!(reloaded.records[0].lines[4..], opaque);
        assert_eq!(reloaded.records[5].lines, untouched);
        assert_eq!(reloaded.active, [0, 5]);
        assert!(
            loaded.records[0]
                .set_identity("Name\nInjected", "", "")
                .is_err()
        );
    }
}

/// Stock AI/Teams.txt uses indexed records instead of the user's active-list header.
pub fn decode_opponents(bytes: &[u8]) -> Vec<(usize, Team)> {
    let lines = bytes
        .split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
        .collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            String::from_utf8_lossy(l)
                .strip_prefix("index ")
                .and_then(|s| s.trim().parse::<usize>().ok())
                .map(|id| (i, id))
        })
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .filter_map(|(n, (start, id))| {
            let end = starts.get(n + 1).map_or(lines.len(), |p| p.0);
            (end - start >= 5).then(|| {
                (
                    *id,
                    Team {
                        lines: lines[start + 1..end].to_vec(),
                    },
                )
            })
        })
        .collect()
}
