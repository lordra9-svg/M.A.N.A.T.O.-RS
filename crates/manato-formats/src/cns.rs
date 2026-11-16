use std::collections::HashMap;
use std::path::Path;

use crate::text::{parse_pair, TextFile};

/// Character constants (`[Data]`, `[Size]`, `[Velocity]`, `[Movement]`, ...).
///
/// Keys are normalised to lowercase `section.key` so `Const(velocity.walk.fwd.x)`
/// can be resolved with a single lookup. `[Data]` entries are also available
/// without a prefix.
#[derive(Debug, Clone, Default)]
pub struct Constants {
    values: HashMap<String, f32>,
    raw: HashMap<String, String>,
}

impl Constants {
    /// Raw string entries (unquoted), keyed by their section-prefixed name.
    pub fn raw_entries(&self) -> impl Iterator<Item = (&String, &String)> {
        self.raw.iter()
    }

    pub fn get(&self, key: &str) -> Option<f32> {
        let k = key.trim().to_ascii_lowercase();
        self.values
            .get(&k)
            .copied()
            .or_else(|| {
                // Accept a leading "data." prefix for values declared in [Data].
                k.strip_prefix("data.")
                    .and_then(|rest| self.values.get(rest).copied())
            })
            .or_else(|| {
                // Accept unprefixed lookups for [Data] values.
                self.values.get(&format!("data.{k}")).copied()
            })
    }

    pub fn get_or(&self, key: &str, default: f32) -> f32 {
        self.get(key).unwrap_or(default)
    }

    pub fn raw(&self, key: &str) -> Option<&str> {
        self.raw
            .get(&key.trim().to_ascii_lowercase())
            .map(|s| s.as_str())
    }

    pub fn insert(&mut self, key: impl Into<String>, value: f32) {
        self.values.insert(key.into().to_ascii_lowercase(), value);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &f32)> {
        self.values.iter()
    }
}

/// One state controller inside a `[State ...]` section.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Controller {
    /// Controller type, lowercased (e.g. `changesate`, `velset`).
    pub kind: String,
    /// All parameters in file order, keys lowercased.
    pub params: Vec<(String, String)>,
    pub line: usize,
}

impl Controller {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(|v| v.trim().parse().ok())
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key)
            .and_then(|v| v.trim().parse().ok())
            .or_else(|| self.get_f32(key).map(|f| f as i32))
    }

    pub fn get_pair(&self, key: &str) -> Option<(f32, f32)> {
        self.get(key).map(parse_pair)
    }

    pub fn get_pair_i32(&self, key: &str) -> Option<(i32, i32)> {
        let (x, y) = self.get_pair(key)?;
        Some((x as i32, y as i32))
    }

    /// `triggerall` expressions plus the numbered trigger groups.
    pub fn trigger_groups(&self) -> (Vec<&str>, Vec<(u32, Vec<&str>)>) {
        let mut all = Vec::new();
        let mut numbered: Vec<(u32, Vec<&str>)> = Vec::new();
        for (k, v) in &self.params {
            if k == "triggerall" {
                all.push(v.as_str());
            } else if let Some(rest) = k.strip_prefix("trigger") {
                if let Ok(n) = rest.parse::<u32>() {
                    match numbered.iter_mut().find(|(num, _)| *num == n) {
                        Some((_, list)) => list.push(v.as_str()),
                        None => numbered.push((n, vec![v.as_str()])),
                    }
                }
            }
        }
        numbered.sort_by_key(|(n, _)| *n);
        (all, numbered)
    }

    pub fn persistent(&self) -> i32 {
        self.get_i32("persistent").unwrap_or(1)
    }

    pub fn ignore_hit_pause(&self) -> bool {
        self.get_i32("ignorehitpause").unwrap_or(0) != 0
    }
}

/// A `[Statedef n]` plus its controllers.
#[derive(Debug, Clone, Default)]
pub struct StateDef {
    pub number: i32,
    pub params: Vec<(String, String)>,
    pub controllers: Vec<Controller>,
    pub line: usize,
    /// Path of the file this state was defined in (debugging).
    pub source: String,
}

impl StateDef {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.trim().parse().ok())
    }

    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(|v| v.trim().parse().ok())
    }

    pub fn get_pair(&self, key: &str) -> Option<(f32, f32)> {
        self.get(key).map(parse_pair)
    }
}

/// A CNS / state file.
#[derive(Debug, Clone, Default)]
pub struct StateFile {
    pub states: Vec<StateDef>,
    pub constants: Constants,
}

impl StateFile {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = TextFile::load(path)?;
        Ok(Self::from_text(&text, &path.display().to_string()))
    }

    pub fn from_text(text: &TextFile, source: &str) -> Self {
        let mut file = StateFile::default();
        let mut current_state: Option<usize> = None;

        for section in &text.sections {
            let header = section.header_lower();
            let kind = section.kind();
            match kind.as_str() {
                "statedef" => {
                    let number = header
                        .strip_prefix("statedef")
                        .and_then(|rest| rest.trim().parse::<i32>().ok())
                        .unwrap_or(0);
                    let params = section
                        .entries
                        .iter()
                        .map(|entry| {
                            (
                                entry.key.trim().to_ascii_lowercase(),
                                entry.value.trim().to_string(),
                            )
                        })
                        .collect();
                    file.states.push(StateDef {
                        number,
                        params,
                        controllers: Vec::new(),
                        line: section.line,
                        source: source.to_string(),
                    });
                    current_state = Some(file.states.len() - 1);
                }
                "state" => {
                    let label = header
                        .split_whitespace()
                        .nth(1)
                        .and_then(|value| value.trim_end_matches(',').parse::<i32>().ok())
                        .unwrap_or(0);
                    let params: Vec<(String, String)> = section
                        .entries
                        .iter()
                        .map(|entry| {
                            (
                                entry.key.trim().to_ascii_lowercase(),
                                entry.value.trim().to_string(),
                            )
                        })
                        .collect();
                    let kind_name = params
                        .iter()
                        .find(|(key, _)| key == "type")
                        .map(|(_, value)| value.to_ascii_lowercase())
                        .unwrap_or_else(|| "null".into());
                    let controller = Controller {
                        kind: kind_name,
                        params,
                        line: section.line,
                    };
                    // Controllers belong to the enclosing Statedef. CMD files
                    // have no Statedef, so the label (usually -1) creates or
                    // selects the state there.
                    let index = if let Some(current) = current_state {
                        Some(current)
                    } else {
                        file.states
                            .iter()
                            .rposition(|state| state.number == label)
                            .or_else(|| {
                                file.states.push(StateDef {
                                    number: label,
                                    params: Vec::new(),
                                    controllers: Vec::new(),
                                    line: section.line,
                                    source: source.to_string(),
                                });
                                Some(file.states.len() - 1)
                            })
                    };
                    if let Some(index) = index {
                        file.states[index].controllers.push(controller);
                        current_state = Some(index);
                    }
                }
                _ if !section.header.trim().is_empty() => {
                    let prefix = header.split_whitespace().next().unwrap_or("").to_string();
                    for entry in &section.entries {
                        let key = entry.key.trim().to_ascii_lowercase();
                        if key.is_empty() {
                            continue;
                        }
                        let full = if prefix == "data" {
                            key.clone()
                        } else {
                            format!("{prefix}.{key}")
                        };
                        if let Ok(value) = entry.value.trim().parse::<f32>() {
                            file.constants.insert(full.clone(), value);
                            if prefix == "velocity" {
                                file.constants.insert(format!("{full}.x"), value);
                                file.constants.insert(format!("{full}.y"), 0.0);
                            }
                            if prefix == "data" {
                                file.constants.insert(format!("data.{full}"), value);
                            }
                        } else if entry.value.contains(',') {
                            let (x, y) = parse_pair(&entry.value);
                            file.constants.insert(format!("{full}.x"), x);
                            file.constants.insert(format!("{full}.y"), y);
                        }
                        file.constants
                            .raw
                            .insert(full.to_ascii_lowercase(), entry.value.clone());
                    }
                }
                _ => {}
            }
        }
        file
    }

    pub fn state(&self, number: i32) -> Option<&StateDef> {
        self.states
            .iter()
            .rev()
            .find(|state| state.number == number)
    }

    pub fn merge(&mut self, other: StateFile) {
        // M.U.G.E.N load order: stcommon -> cns -> st -> st1..st9 -> cmd.
        for (key, value) in other.constants.values {
            self.constants.values.insert(key, value);
        }
        for (key, value) in other.constants.raw {
            self.constants.raw.insert(key, value);
        }
        for state in other.states {
            if let Some(existing) = self
                .states
                .iter_mut()
                .rposition(|candidate| candidate.number == state.number)
                .map(|index| &mut self.states[index])
            {
                *existing = state;
            } else {
                self.states.push(state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[Data]
life = 1000
[Velocity]
walk.fwd.x = 2.4
[Statedef 0]
type = S
physics = S
[State 0, stop]
type = VelSet
trigger1 = Time = 0
trigger2 = Command = "holdfwd"
x = 0
[State 0, next]
type = ChangeState
trigger1 = AnimTime = 0
value = 20
"#;

    #[test]
    fn parses_constants_and_states() {
        let tf = TextFile::parse_str(SAMPLE);
        let sf = StateFile::from_text(&tf, "test.cns");
        assert_eq!(sf.constants.get("life"), Some(1000.0));
        assert_eq!(sf.constants.get("velocity.walk.fwd.x"), Some(2.4));
        let s = sf.state(0).unwrap();
        assert_eq!(s.get("type"), Some("S"));
        assert_eq!(s.controllers.len(), 2);
        assert_eq!(s.controllers[0].kind, "velset");
        let (all, groups) = s.controllers[0].trigger_groups();
        assert!(all.is_empty());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1, vec!["Time = 0"]);
    }

    #[test]
    fn controllers_belong_to_the_enclosing_statedef() {
        // `[State 300]` inside `[Statedef -2]` is a label, not state 300.
        let tf = TextFile::parse_str(
            r#"
[Statedef -2]
[State 300]
type = VarSet
trigger1 = 1
var(52) = 3
"#,
        );
        let sf = StateFile::from_text(&tf, "option.txt");
        assert!(sf.state(300).is_none());
        let special = sf.state(-2).expect("special state");
        assert_eq!(special.controllers.len(), 1);
        assert_eq!(special.controllers[0].kind, "varset");
    }
}
