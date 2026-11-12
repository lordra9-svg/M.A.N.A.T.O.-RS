use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::air::Animation;
use crate::cmd::{CmdFile, CommandDef};
use crate::cns::{Constants, StateDef, StateFile};
use crate::def::CharacterDef;
use crate::sff::SffFile;
use crate::snd::SoundBank;

/// A fully loaded character: metadata, scripts, sprites and sounds.
#[derive(Debug)]
pub struct Character {
    pub def: CharacterDef,
    pub constants: Constants,
    /// Default-language victory quotes (`[Quotes]`), sorted by index.
    pub quotes: Vec<(i32, String)>,
    pub states: HashMap<i32, StateDef>,
    pub commands: Vec<CommandDef>,
    pub animations: HashMap<i32, Animation>,
    pub sprites: SffFile,
    pub sounds: SoundBank,
    pub state_order: Vec<i32>,
}

impl Character {
    pub fn load(def_path: &Path, data_dirs: &[PathBuf]) -> anyhow::Result<Self> {
        let def = CharacterDef::load(def_path)?;
        let resolve = |path: &Path| -> Option<PathBuf> {
            if path.exists() {
                return Some(path.to_path_buf());
            }
            let name = path.file_name()?;
            for dir in data_dirs {
                let candidate = dir.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            None
        };

        let mut merged = StateFile::default();
        let mut state_order = Vec::new();

        // Common states first, then character-specific state files.
        let mut ordered: Vec<&PathBuf> = Vec::new();
        ordered.extend(def.files.common_states.iter());
        if let Some(cns) = &def.files.cns {
            ordered.push(cns);
        }
        ordered.extend(def.files.states.iter());

        for path in ordered {
            let Some(found) = resolve(path) else {
                log::warn!("state file not found: {}", path.display());
                continue;
            };
            let file = StateFile::load(&found)
                .with_context(|| format!("failed to load state file {}", found.display()))?;
            state_order.extend(file.states.iter().map(|s| s.number));
            merged.merge(file);
        }

        let mut cmd = CmdFile::default();
        if let Some(path) = &def.files.cmd {
            if let Some(found) = resolve(path) {
                cmd = CmdFile::load(&found)
                    .with_context(|| format!("failed to load cmd file {}", found.display()))?;
                state_order.extend(cmd.states.states.iter().map(|s| s.number));
                merged.merge(cmd.states.clone());
            } else {
                log::warn!("cmd file not found: {}", path.display());
            }
        }

        let mut animations = HashMap::new();
        if let Some(path) = &def.files.anim {
            if let Some(found) = resolve(path) {
                match crate::air::parse_air(&found) {
                    Ok(anims) => {
                        for a in anims {
                            animations.insert(a.number, a);
                        }
                    }
                    Err(e) => log::warn!("failed to load AIR {}: {e}", found.display()),
                }
            }
        }

        let sprites = match &def.files.sprite {
            Some(path) => match resolve(path) {
                Some(found) => SffFile::load(&found)
                    .with_context(|| format!("failed to load SFF {}", found.display()))?,
                None => {
                    log::warn!("sprite file not found: {}", path.display());
                    SffFile::default()
                }
            },
            None => SffFile::default(),
        };

        let sounds = match &def.files.sound {
            Some(path) => match resolve(path) {
                Some(found) => SoundBank::load(&found)
                    .with_context(|| format!("failed to load SND {}", found.display()))?,
                None => SoundBank::default(),
            },
            None => SoundBank::default(),
        };

        let mut states = HashMap::new();
        for state in merged.states {
            states.insert(state.number, state);
        }
        let quotes = parse_quotes(&merged.constants);

        Ok(Self {
            def,
            constants: merged.constants,
            quotes,
            states,
            commands: cmd.commands,
            animations,
            sprites,
            sounds,
            state_order,
        })
    }

    pub fn state(&self, number: i32) -> Option<&StateDef> {
        self.states.get(&number)
    }

    pub fn animation(&self, number: i32) -> Option<&Animation> {
        self.animations.get(&number)
    }

    pub fn name(&self) -> &str {
        &self.def.name
    }

    pub fn display_name(&self) -> &str {
        &self.def.display_name
    }

    pub fn localcoord(&self) -> (f32, f32) {
        self.def.localcoord
    }
}

/// Extracts the default-language `[Quotes]` victory entries (`victory0` …
/// `victory99`), ignoring alternate-language sections.
fn parse_quotes(constants: &Constants) -> Vec<(i32, String)> {
    let mut quotes: Vec<(i32, String)> = constants
        .raw_entries()
        .filter_map(|(key, value)| {
            let index = key.strip_prefix("quotes.victory")?.parse::<i32>().ok()?;
            if !(0..100).contains(&index) || value.trim().is_empty() {
                return None;
            }
            Some((index, value.trim().to_string()))
        })
        .collect();
    quotes.sort_by_key(|(index, _)| *index);
    quotes
}

/// Shared effect sprites and animations (`data/fightfx.sff` + `fightfx.air`).
#[derive(Debug, Clone, Default)]
pub struct EffectsBank {
    pub sprites: SffFile,
    pub animations: HashMap<i32, Animation>,
}

impl EffectsBank {
    pub fn load(data_dirs: &[PathBuf]) -> Self {
        for dir in data_dirs {
            let sff_path = dir.join("fightfx.sff");
            let air_path = dir.join("fightfx.air");
            if sff_path.exists() && air_path.exists() {
                let sprites = SffFile::load(&sff_path).unwrap_or_else(|e| {
                    log::warn!("failed to load fightfx.sff: {e}");
                    SffFile::default()
                });
                let animations = crate::air::parse_air(&air_path)
                    .unwrap_or_else(|e| {
                        log::warn!("failed to load fightfx.air: {e}");
                        Vec::new()
                    })
                    .into_iter()
                    .map(|a| (a.number, a))
                    .collect();
                return Self {
                    sprites,
                    animations,
                };
            }
        }
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_quotes;
    use crate::cns::StateFile;
    use crate::text::TextFile;

    #[test]
    fn extracts_default_language_victory_quotes() {
        let text = TextFile::parse_str(
            r#"
[Quotes]
victory0 = "First quote."
victory2 = Second quote.
victory99 = "Last quote."
[ja.Quotes]
victory1 = "Japanese quote."
[Data]
life = 1000
"#,
        );
        let file = StateFile::from_text(&text, "quotes.cns");
        let quotes = parse_quotes(&file.constants);
        assert_eq!(
            quotes,
            vec![
                (0, "First quote.".to_string()),
                (2, "Second quote.".to_string()),
                (99, "Last quote.".to_string()),
            ]
        );
    }
}
