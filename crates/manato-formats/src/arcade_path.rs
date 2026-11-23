//! Restricted reader for Ikemen-GO arcade path Lua files.
//!
//! Arcade paths are ordinary Lua scripts that call `launchFight{...}` for
//! each match. M.A.N.A.T.O.-RS executes them in a sandbox that exposes only
//! the path helpers (`launchFight`, `launchStoryboard`, `matchNo`,
//! `continued`, `getConsecutiveWins`, `setMatchNo`) plus the Lua string,
//! table, and math libraries. The collected fights are resolved against the
//! select.def roster by the front end.
//!
//! Supported `launchFight` fields: `p2char`, `p2numchars`, `p2teammode`,
//! `p2rounds`, `order`, `exclude`, `stage`, `time`, `music`, `ai`. Fields are
//! retained on [`ArcadePathFight`] but only roster resolution, order, exclude,
//! and time are consumed by the current app flow; visual/audio overrides are
//! documented limitations.

use std::path::Path;

use anyhow::Context as _;
use mlua::{Lua, LuaOptions, StdLib, Table, Value};

/// One `launchFight{}` entry from an arcade path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArcadePathFight {
    /// `p2char` names/paths; empty means "random roster opponent".
    pub p2char: Vec<String>,
    pub p2_numchars: Option<i32>,
    pub p2_teammode: Option<String>,
    pub p2_rounds: Option<i32>,
    pub order: Option<i32>,
    pub exclude: Vec<String>,
    pub stage: Option<String>,
    pub time: Option<i32>,
    pub mode: Option<String>,
    pub music: Option<String>,
    pub ai: Option<f32>,
}

const MAX_FIGHTS: usize = 64;

pub fn load(path: &Path) -> anyhow::Result<Vec<ArcadePathFight>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read arcade path {}", path.display()))?;
    run(&source).with_context(|| format!("arcade path {}", path.display()))
}

/// Executes a restricted arcade path and returns the requested fights.
pub fn run(source: &str) -> anyhow::Result<Vec<ArcadePathFight>> {
    run_lua(source).map_err(|error| anyhow::anyhow!("{error}"))
}

fn run_lua(source: &str) -> mlua::Result<Vec<ArcadePathFight>> {
    let lua = Lua::new_with(
        StdLib::STRING | StdLib::TABLE | StdLib::MATH,
        LuaOptions::default(),
    )?;
    let fights: std::rc::Rc<std::cell::RefCell<Vec<ArcadePathFight>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

    let globals = lua.globals();
    {
        let fights = fights.clone();
        globals.set(
            "launchFight",
            lua.create_function(move |_, table: Option<Table>| {
                if fights.borrow().len() >= MAX_FIGHTS {
                    return Ok(false);
                }
                fights
                    .borrow_mut()
                    .push(parse_fight(table.as_ref()).unwrap_or_default());
                Ok(true)
            })?,
        )?;
    }
    globals.set(
        "launchStoryboard",
        lua.create_function(|_, _path: String| Ok(true))?,
    )?;
    globals.set("setMatchNo", lua.create_function(|_, _: i32| Ok(()))?)?;
    {
        let fights = fights.clone();
        globals.set(
            "matchNo",
            lua.create_function(move |_, ()| Ok(fights.borrow().len() as i32 + 1))?,
        )?;
    }
    globals.set("continued", lua.create_function(|_, ()| Ok(false))?)?;
    globals.set(
        "getConsecutiveWins",
        lua.create_function(|_, _: Option<i32>| Ok(0))?,
    )?;

    lua.load(source).set_name("arcadepath").exec()?;
    let fights = fights.borrow().clone();
    Ok(fights)
}

fn parse_fight(table: Option<&Table>) -> Option<ArcadePathFight> {
    let table = table?;
    Some(ArcadePathFight {
        p2char: string_list(table.get("p2char").ok()),
        exclude: string_list(table.get("exclude").ok()),
        p2_numchars: int_value(table.get("p2numchars").ok()),
        p2_rounds: int_value(table.get("p2rounds").ok()),
        order: int_value(table.get("order").ok()),
        time: int_value(table.get("time").ok()),
        p2_teammode: string_value(table.get("p2teammode").ok()),
        mode: string_value(table.get("mode").ok()),
        stage: string_value(table.get("stage").ok()),
        music: string_value(table.get("music").ok()),
        ai: table
            .get::<Option<f64>>("ai")
            .ok()
            .flatten()
            .map(|v| v as f32),
    })
}

fn string_list(value: Option<Value>) -> Vec<String> {
    match value {
        Some(Value::Table(table)) => {
            let mut out = Vec::new();
            for entry in table.sequence_values::<String>().flatten() {
                out.push(entry);
            }
            out
        }
        Some(Value::String(text)) => text
            .to_str()
            .map(|text| vec![text.to_string()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn string_value(value: Option<Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(text) => text.to_str().ok().map(|text| text.to_string()),
        _ => None,
    })
}

fn int_value(value: Option<Value>) -> Option<i32> {
    value.and_then(|value| match value {
        Value::Integer(number) => Some(number as i32),
        Value::Number(number) => Some(number as i32),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_linear_path_with_rival_and_order() {
        let fights = run(
            "launchStoryboard('intro.def')\n\
             for i = 1, 2 do if not launchFight{} then return end end\n\
             if not launchFight{p2char = {'chars/rival/rival.def'}, order = 3, stage = 'stages/dojo.def'} then return end\n\
             setMatchNo(-1)\n",
        )
        .expect("path");
        assert_eq!(fights.len(), 3);
        assert!(fights[0].p2char.is_empty());
        assert_eq!(fights[2].p2char, ["chars/rival/rival.def"]);
        assert_eq!(fights[2].order, Some(3));
        assert_eq!(fights[2].stage.as_deref(), Some("stages/dojo.def"));
    }

    #[test]
    fn supports_match_no_branches_and_excludes() {
        let fights = run(
            "if matchNo() == 1 then launchFight{exclude = {'Boss'}} end\n\
             if matchNo() == 2 then launchFight{p2char = {'Boss'}, p2teammode = 'simul', p2numchars = 2} end\n",
        )
        .expect("path");
        assert_eq!(fights.len(), 2);
        assert_eq!(fights[0].exclude, ["Boss"]);
        assert_eq!(fights[1].p2char, ["Boss"]);
        assert_eq!(fights[1].p2_teammode.as_deref(), Some("simul"));
        assert_eq!(fights[1].p2_numchars, Some(2));
    }

    #[test]
    fn rejects_unavailable_libraries() {
        assert!(run("os.execute('true')").is_err());
    }
}
