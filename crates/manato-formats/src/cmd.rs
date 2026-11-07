use std::path::Path;

use manato_core::{Buttons, Dir8};

use crate::cns::StateFile;
use crate::text::TextFile;

/// One step of a command motion (`~D`, `$F`, `x+y`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommandStep {
    pub dir: Option<Dir8>,
    pub buttons: u8,
    /// `~` prefix: the input must have been released.
    pub release: bool,
    /// `/` prefix (or `$`): the direction must still be held.
    pub hold: bool,
    /// `$` prefix: match the direction as one of the four cardinal groups.
    pub pool4: bool,
    /// `~` applying to a button: the button must have been released.
    pub button_release: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CommandDef {
    pub name: String,
    pub steps: Vec<CommandStep>,
    /// Maximum number of ticks the whole motion may take.
    pub time: i32,
    pub buffer_time: i32,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct CmdFile {
    pub commands: Vec<CommandDef>,
    pub states: StateFile,
    pub defaults: Vec<(String, String)>,
}

impl CmdFile {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = TextFile::load(path)?;
        Ok(Self::from_text(&text, &path.display().to_string()))
    }

    pub fn from_text(text: &TextFile, source: &str) -> Self {
        let mut commands = Vec::new();
        let mut defaults = Vec::new();
        for section in &text.sections {
            if section.kind() == "command" {
                let name = section.get("name").unwrap_or("").to_string();
                let sequence = section.get("command").unwrap_or("");
                let time = section
                    .get("time")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(15);
                let buffer_time = section
                    .get("buffer.time")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(1);
                let steps = sequence
                    .split(',')
                    .filter_map(parse_step)
                    .collect::<Vec<_>>();
                if !name.is_empty() && !steps.is_empty() {
                    commands.push(CommandDef {
                        name,
                        steps,
                        time,
                        buffer_time,
                        line: section.line,
                    });
                }
            } else if section.kind() == "defaults" {
                defaults.extend(
                    section
                        .entries
                        .iter()
                        .map(|e| (e.key.to_ascii_lowercase(), e.value.clone())),
                );
            }
        }
        let states = StateFile::from_text(text, source);
        Self {
            commands,
            states,
            defaults,
        }
    }

    pub fn command(&self, name: &str) -> Option<&CommandDef> {
        self.commands.iter().find(|c| c.name == name)
    }
}

fn parse_step(raw: &str) -> Option<CommandStep> {
    let mut token = raw.trim();
    if token.is_empty() {
        return None;
    }
    let mut step = CommandStep::default();
    loop {
        if let Some(rest) = token.strip_prefix('~') {
            step.release = true;
            token = rest;
        } else if let Some(rest) = token.strip_prefix('$') {
            step.pool4 = true;
            token = rest;
        } else if let Some(rest) = token.strip_prefix('/') {
            step.hold = true;
            token = rest;
        } else {
            break;
        }
    }
    for part in token.split('+') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some(dir) = parse_dir(p) {
            step.dir = Some(dir);
        }
        let lower = p.to_ascii_lowercase();
        if lower == "start" || lower == "s" {
            step.buttons |= Buttons::START;
        } else if let Some(c) = lower.chars().next() {
            if let Some(btn) = Buttons::from_letter(c) {
                step.buttons |= btn;
            }
        }
    }
    if step.buttons != 0 && step.release {
        step.button_release = true;
    }
    if step.dir.is_none() && step.buttons == 0 {
        None
    } else {
        Some(step)
    }
}

fn parse_dir(s: &str) -> Option<Dir8> {
    match s.to_ascii_uppercase().as_str() {
        "U" => Some(Dir8::Up),
        "UF" => Some(Dir8::UpFwd),
        "F" => Some(Dir8::Fwd),
        "DF" => Some(Dir8::DnFwd),
        "D" => Some(Dir8::Down),
        "DB" => Some(Dir8::DnBack),
        "B" => Some(Dir8::Back),
        "UB" => Some(Dir8::UpBack),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_steps() {
        let s = parse_step("~D").unwrap();
        assert_eq!(s.dir, Some(Dir8::Down));
        assert!(s.release);
        let s = parse_step("$F").unwrap();
        assert!(s.pool4);
        let s = parse_step("/F").unwrap();
        assert!(s.hold);
        let s = parse_step("x+y").unwrap();
        assert_eq!(s.buttons, Buttons::X | Buttons::Y);
    }
}
