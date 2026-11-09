//! M.U.G.E.N/Ikemen-GO CMD command definitions.
//!
//! The command grammar and matching rules are specified in
//! `docs/spec/cns-cmd.md`.

use std::path::Path;

use manato_core::{Buttons, Dir8};

use crate::cns::StateFile;
use crate::text::TextFile;

/// One step of a command motion (`~D`, `$F`, `x+y`, `a|b`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommandStep {
    pub dir: Option<Dir8>,
    /// Buttons that must all be pressed.
    pub buttons: u8,
    /// Buttons of which at least one must be pressed (`|` syntax).
    pub or_buttons: u8,
    /// `~` prefix: the input must have been released.
    pub release: bool,
    /// `/` prefix: the direction must still be held.
    pub hold: bool,
    /// `$` prefix: match the direction as one of the four cardinal groups.
    pub pool4: bool,
    /// `~` applying to a button: the button must have been released.
    pub button_release: bool,
    /// `>` prefix: greater-direction expansion.
    pub greater: bool,
    /// `*` prefix: exact-direction tap.
    pub star: bool,
    /// Charge duration in ticks after `/` or `~`.
    pub charge_time: i32,
}

#[derive(Debug, Clone, Default)]
pub struct CommandDef {
    pub name: String,
    pub steps: Vec<CommandStep>,
    /// Maximum number of ticks the whole motion may take.
    pub time: i32,
    /// Maximum ticks between steps; `-1` means `time`.
    pub step_time: i32,
    pub buffer_time: i32,
    pub auto_greater: bool,
    pub buffer_hitpause: bool,
    pub buffer_pauseend: bool,
    pub buffer_shared: bool,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct CommandDefaults {
    pub time: i32,
    pub step_time: i32,
    pub buffer_time: i32,
    pub auto_greater: bool,
    pub buffer_hitpause: bool,
    pub buffer_pauseend: bool,
    pub buffer_shared: bool,
}

impl Default for CommandDefaults {
    fn default() -> Self {
        Self {
            time: 15,
            step_time: -1,
            buffer_time: 1,
            auto_greater: false,
            buffer_hitpause: false,
            buffer_pauseend: false,
            buffer_shared: false,
        }
    }
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
        let mut command_defaults = CommandDefaults::default();

        for section in &text.sections {
            if section.kind() == "defaults" {
                defaults.extend(
                    section
                        .entries
                        .iter()
                        .map(|entry| (entry.key.to_ascii_lowercase(), entry.value.clone())),
                );
                for (key, value) in &defaults {
                    match key.as_str() {
                        "command.time" => {
                            if let Ok(time) = value.trim().parse::<i32>() {
                                command_defaults.time = time;
                            }
                        }
                        "command.steptime" => {
                            if let Ok(time) = value.trim().parse::<i32>() {
                                command_defaults.step_time = time;
                            }
                        }
                        "command.buffer.time" => {
                            if let Ok(time) = value.trim().parse::<i32>() {
                                command_defaults.buffer_time = time.max(1);
                            }
                        }
                        "command.autogreater" => {
                            command_defaults.auto_greater = value.trim() != "0";
                        }
                        "command.buffer.hitpause" => {
                            command_defaults.buffer_hitpause = value.trim() != "0";
                        }
                        "command.buffer.pauseend" => {
                            command_defaults.buffer_pauseend = value.trim() != "0";
                        }
                        "command.buffer.shared" => {
                            command_defaults.buffer_shared = value.trim() != "0";
                        }
                        _ => {}
                    }
                }
                continue;
            }
            if section.kind() == "command" {
                let name = section.get("name").unwrap_or("").trim().to_string();
                let sequence = section.get("command").unwrap_or("");
                let time = section
                    .get("time")
                    .and_then(|value| value.trim().parse::<i32>().ok())
                    .unwrap_or(command_defaults.time);
                let step_time = section
                    .get("steptime")
                    .and_then(|value| value.trim().parse::<i32>().ok())
                    .unwrap_or(command_defaults.step_time);
                let buffer_time = section
                    .get("buffer.time")
                    .and_then(|value| value.trim().parse::<i32>().ok())
                    .map(|value| value.max(1))
                    .unwrap_or(command_defaults.buffer_time);
                let auto_greater = section
                    .get("autogreater")
                    .map(|value| value.trim() != "0")
                    .unwrap_or(command_defaults.auto_greater);
                let buffer_hitpause = section
                    .get("buffer.hitpause")
                    .map(|value| value.trim() != "0")
                    .unwrap_or(command_defaults.buffer_hitpause);
                let buffer_pauseend = section
                    .get("buffer.pauseend")
                    .map(|value| value.trim() != "0")
                    .unwrap_or(command_defaults.buffer_pauseend);
                let buffer_shared = section
                    .get("buffer.shared")
                    .map(|value| value.trim() != "0")
                    .unwrap_or(command_defaults.buffer_shared);
                let steps = sequence
                    .split(',')
                    .filter_map(parse_step)
                    .collect::<Vec<_>>();
                if !name.is_empty() && !steps.is_empty() {
                    commands.push(CommandDef {
                        name,
                        steps,
                        time,
                        step_time,
                        buffer_time,
                        auto_greater,
                        buffer_hitpause,
                        buffer_pauseend,
                        buffer_shared,
                        line: section.line,
                    });
                }
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
        self.commands
            .iter()
            .find(|command| command.name.eq_ignore_ascii_case(name))
    }
}

fn parse_step(raw: &str) -> Option<CommandStep> {
    let token = raw.trim();
    if token.is_empty() {
        return None;
    }

    let or_logic = token.contains('|');
    let parts: Vec<&str> = if or_logic {
        token.split('|').map(str::trim).collect()
    } else {
        token.split('+').map(str::trim).collect()
    };

    let mut step = CommandStep::default();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let (prefix, key) = parse_prefix(part, &mut step)?;
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        if let Some(dir) = parse_dir(key) {
            if step.dir.is_none() {
                step.dir = Some(dir);
            }
        } else if let Some(button) = parse_button(key) {
            if or_logic {
                step.or_buttons |= button;
            } else {
                step.buttons |= button;
            }
            if step.release {
                step.button_release = true;
            }
        } else {
            // Unknown token: keep parsing other parts.
            let _ = prefix;
        }
    }

    if step.dir.is_none() && step.buttons == 0 && step.or_buttons == 0 {
        None
    } else {
        Some(step)
    }
}

fn parse_prefix<'a>(part: &'a str, step: &mut CommandStep) -> Option<(String, &'a str)> {
    let mut rest = part.trim();
    let mut prefix = String::new();
    while let Some(ch) = rest.chars().next() {
        match ch {
            '~' | '/' | '$' | '>' | '*' => {
                prefix.push(ch);
                rest = &rest[ch.len_utf8()..];
                // Optional charge duration after ~ or /.
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if !digits.is_empty() {
                    if let Ok(value) = digits.parse::<i32>() {
                        step.charge_time = value;
                    }
                    rest = &rest[digits.len()..];
                }
            }
            _ => break,
        }
    }
    for ch in prefix.chars() {
        match ch {
            '~' => step.release = true,
            '/' => step.hold = true,
            '$' => step.pool4 = true,
            '>' => step.greater = true,
            '*' => step.star = true,
            _ => {}
        }
    }
    Some((prefix, rest))
}

fn parse_dir(value: &str) -> Option<Dir8> {
    match value {
        "U" => Some(Dir8::Up),
        "UF" | "UR" => Some(Dir8::UpFwd),
        "F" | "R" => Some(Dir8::Fwd),
        "DF" | "DR" => Some(Dir8::DnFwd),
        "D" => Some(Dir8::Down),
        "DB" | "DL" => Some(Dir8::DnBack),
        "B" | "L" => Some(Dir8::Back),
        "UB" | "UL" => Some(Dir8::UpBack),
        "N" => Some(Dir8::Neutral),
        _ => None,
    }
}

fn parse_button(value: &str) -> Option<u8> {
    if value == "start" || value == "s" {
        return Some(Buttons::START);
    }
    let mut button = 0u8;
    for ch in value.chars() {
        button |= Buttons::from_letter(ch)?;
    }
    (button != 0).then_some(button)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_steps() {
        let step = parse_step("~D").unwrap();
        assert_eq!(step.dir, Some(Dir8::Down));
        assert!(step.release);
        let step = parse_step("$F").unwrap();
        assert!(step.pool4);
        let step = parse_step("/F").unwrap();
        assert!(step.hold);
        let step = parse_step("x+y").unwrap();
        assert_eq!(step.buttons, Buttons::X | Buttons::Y);
        let step = parse_step("*F").unwrap();
        assert!(step.star);
        let step = parse_step(">F").unwrap();
        assert!(step.greater);
        let step = parse_step("~x").unwrap();
        assert!(step.button_release);
        assert_eq!(step.buttons, Buttons::X);
    }

    #[test]
    fn parses_or_and_charge_prefixes() {
        let step = parse_step("a|b").unwrap();
        assert_eq!(step.or_buttons, Buttons::A | Buttons::B);
        let step = parse_step("/12D").unwrap();
        assert!(step.hold);
        assert_eq!(step.charge_time, 12);
    }

    #[test]
    fn parses_defaults() {
        let text = TextFile::parse_str(
            "[Defaults]\ncommand.time = 20\ncommand.buffer.time = 3\ncommand.steptime = 7\ncommand.autogreater = 1\n             [Command]\nname = \"x\"\ncommand = x\n             [Command]\nname = \"y\"\ncommand = D, F\nsteptime = 2\nautogreater = 0",
        );
        let file = CmdFile::from_text(&text, "test.cmd");
        let command = file.command("x").unwrap();
        assert_eq!(command.time, 20);
        assert_eq!(command.buffer_time, 3);
        assert_eq!(command.step_time, 7);
        assert!(command.auto_greater);
        let overridden = file.command("y").unwrap();
        assert_eq!(overridden.step_time, 2);
        assert!(!overridden.auto_greater);
    }
}
