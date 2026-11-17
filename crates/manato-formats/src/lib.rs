//! Parsers for the M.U.G.E.N content formats used by the engine.
//!
//! The parsers are tolerant: real community content contains typos, mixed case,
//! missing optional fields and non-UTF-8 bytes. Anything that cannot be parsed
//! is logged and skipped rather than aborting a match.

pub mod air;
pub mod character;
pub mod cmd;
pub mod cns;
pub mod def;
pub mod font;
pub mod screenpack;
pub mod sff;
pub mod snd;
pub mod stage;
pub mod storyboard;
pub mod storyboard_lua;
pub mod text;

pub use air::{AnimFrame, Animation, CollisionBox, FramePose};
pub use character::{Character, EffectsBank};
pub use cmd::{CmdFile, CommandDef, CommandStep};
pub use cns::{Constants, Controller, StateDef, StateFile};
pub use def::{CharacterDef, StageDefFile};
pub use font::{BitmapFont, Glyph};
pub use screenpack::{
    BarDef, ComboDef, FaceDef, FightDef, Layer, NameDef, Roster, RosterEntry, ScreenBg, Screenpack,
    SelectDef, TextDef, TimeDef, TitleDef, WinIconDef,
};
pub use sff::{SffFile, Sprite};
pub use snd::{Sound, SoundBank};
pub use stage::{BgCtrl, BgLayer, BgScene, Stage};
pub use text::{Entry, Section, TextFile};
