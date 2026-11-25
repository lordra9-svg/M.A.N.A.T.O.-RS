//! Sandboxed Lua scripting for storyboards.
//!
//! Scripts are loaded with only the `table`, `string`, and `math` libraries,
//! a 4 MiB memory cap, and no I/O or package access. A storyboard opts in with
//! `[SceneDef] lua = file.lua`; the script may define
//!
//! ```lua
//! function update(ctx)
//!   ctx.x = ctx.x + math.sin(ctx.time / 10) * 4
//! end
//! ```
//!
//! The context exposes `scene`, `time`, `x`, `y`, `scale_x`, `scale_y`,
//! `facing`, `alpha`, `jump`, and `done`. Changes to the numeric fields are
//! read back into the overlay after every tick; `jump` switches to a named
//! scene.

use std::path::Path;

use mlua::{Lua, LuaOptions, StdLib};

const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;

fn lua_error(error: mlua::Error) -> anyhow::Error {
    anyhow::anyhow!("storyboard lua: {error}")
}

/// Per-tick values shared with a storyboard script.
#[derive(Debug, Clone)]
pub struct StoryboardScriptContext {
    pub scene: usize,
    pub time: i32,
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub facing: i32,
    pub alpha: f32,
    /// Scene name to jump to after this tick.
    pub jump: Option<String>,
    pub done: bool,
}

impl StoryboardScriptContext {
    pub fn new(scene: usize, time: i32, x: f32, y: f32) -> Self {
        Self {
            scene,
            time,
            x,
            y,
            scale_x: 1.0,
            scale_y: 1.0,
            facing: 1,
            alpha: 1.0,
            jump: None,
            done: false,
        }
    }
}

/// A sandboxed Lua runtime owned by one storyboard overlay.
pub struct StoryboardLua {
    lua: Lua,
}

impl std::fmt::Debug for StoryboardLua {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoryboardLua").finish_non_exhaustive()
    }
}

impl StoryboardLua {
    /// Loads a script file with the restricted library set.
    pub fn load_file(path: &Path) -> anyhow::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        if metadata.len() > MAX_SCRIPT_BYTES {
            anyhow::bail!(
                "storyboard lua script is too large ({} bytes)",
                metadata.len()
            );
        }
        let source = std::fs::read_to_string(path)?;
        Self::load(&source, &path.display().to_string())
    }

    pub fn load(source: &str, name: &str) -> anyhow::Result<Self> {
        if source.len() as u64 > MAX_SCRIPT_BYTES {
            anyhow::bail!(
                "storyboard lua script is too large ({} bytes)",
                source.len()
            );
        }
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::default(),
        )
        .map_err(lua_error)?;
        // Bounded memory keeps hostile storyboards from exhausting the host.
        lua.set_memory_limit(4 * 1024 * 1024).map_err(lua_error)?;
        lua.load(source).set_name(name).exec().map_err(lua_error)?;
        Ok(Self { lua })
    }

    /// Runs the optional `update(ctx)` function for one tick.
    pub fn update(&self, ctx: &mut StoryboardScriptContext) -> anyhow::Result<()> {
        let table = self.lua.create_table().map_err(lua_error)?;
        table.set("scene", ctx.scene as i64).map_err(lua_error)?;
        table.set("time", ctx.time).map_err(lua_error)?;
        table.set("x", ctx.x).map_err(lua_error)?;
        table.set("y", ctx.y).map_err(lua_error)?;
        table.set("scale_x", ctx.scale_x).map_err(lua_error)?;
        table.set("scale_y", ctx.scale_y).map_err(lua_error)?;
        table.set("facing", ctx.facing).map_err(lua_error)?;
        table.set("alpha", ctx.alpha).map_err(lua_error)?;
        table.set("done", ctx.done).map_err(lua_error)?;
        let value: mlua::Value = self.lua.globals().get("update").map_err(lua_error)?;
        if let mlua::Value::Function(function) = value {
            function.call::<()>(table.clone()).map_err(lua_error)?;
        }
        ctx.x = table.get::<f32>("x").unwrap_or(ctx.x);
        ctx.y = table.get::<f32>("y").unwrap_or(ctx.y);
        ctx.scale_x = table.get::<f32>("scale_x").unwrap_or(ctx.scale_x);
        ctx.scale_y = table.get::<f32>("scale_y").unwrap_or(ctx.scale_y);
        ctx.facing = table.get::<i32>("facing").unwrap_or(ctx.facing);
        ctx.alpha = table.get::<f32>("alpha").unwrap_or(ctx.alpha);
        ctx.done = table.get::<bool>("done").unwrap_or(ctx.done);
        if let Ok(mlua::Value::String(jump)) = table.get::<mlua::Value>("jump") {
            ctx.jump = jump.to_str().ok().map(|name| name.to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_runs_update_and_reads_back_context() {
        let script = StoryboardLua::load(
            "function update(ctx)\n\
                 ctx.x = ctx.x + math.floor(ctx.time / 2)\n\
                 ctx.scale_x = 2\n\
                 ctx.jump = 'Scene 1'\n\
                 ctx.done = true\n\
             end\n",
            "test.lua",
        )
        .expect("load");
        let mut ctx = StoryboardScriptContext::new(0, 4, 10.0, 20.0);
        script.update(&mut ctx).expect("update");
        assert_eq!(ctx.x, 12.0);
        assert_eq!(ctx.scale_x, 2.0);
        assert_eq!(ctx.jump.as_deref(), Some("Scene 1"));
        assert!(ctx.done);
    }

    #[test]
    fn sandbox_has_no_io_or_os_libraries() {
        assert!(StoryboardLua::load("io.open('x')", "io.lua").is_err());
        assert!(StoryboardLua::load("os.time()", "os.lua").is_err());
        assert!(StoryboardLua::load("require('os')", "req.lua").is_err());
    }

    #[test]
    fn script_without_update_is_allowed() {
        let script = StoryboardLua::load("local x = 1", "noop.lua").expect("load");
        let mut ctx = StoryboardScriptContext::new(0, 0, 5.0, 6.0);
        script.update(&mut ctx).expect("update");
        assert_eq!(ctx.x, 5.0);
    }
}
