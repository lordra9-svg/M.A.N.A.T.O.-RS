use std::fmt;

/// M.U.G.E.N button bitmask. Matches the default six-button layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Buttons;

impl Buttons {
    pub const X: u8 = 1 << 0;
    pub const Y: u8 = 1 << 1;
    pub const Z: u8 = 1 << 2;
    pub const A: u8 = 1 << 3;
    pub const B: u8 = 1 << 4;
    pub const C: u8 = 1 << 5;
    pub const START: u8 = 1 << 6;
    pub const ALL: u8 = Self::X | Self::Y | Self::Z | Self::A | Self::B | Self::C | Self::START;

    pub fn from_letter(c: char) -> Option<u8> {
        match c.to_ascii_lowercase() {
            'x' => Some(Self::X),
            'y' => Some(Self::Y),
            'z' => Some(Self::Z),
            'a' => Some(Self::A),
            'b' => Some(Self::B),
            'c' => Some(Self::C),
            's' => Some(Self::START),
            _ => None,
        }
    }
}

pub const DIR_UP: u8 = 1 << 0;
pub const DIR_DOWN: u8 = 1 << 1;
pub const DIR_LEFT: u8 = 1 << 2;
pub const DIR_RIGHT: u8 = 1 << 3;

/// One frame of player input, as exchanged by rollback netcode.
///
/// Directions are absolute (screen space); command matching converts them to
/// facing-relative directions. Buttons are a [`Buttons`] bitmask.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayerInput {
    pub dir: u8,
    pub buttons: u8,
}

impl PlayerInput {
    pub const fn new(dir: u8, buttons: u8) -> Self {
        Self { dir, buttons }
    }

    pub fn held(&self, button: u8) -> bool {
        self.buttons & button != 0
    }
}

/// Nine-way direction after applying a player's facing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir8 {
    #[default]
    Neutral,
    Up,
    UpFwd,
    Fwd,
    DnFwd,
    Down,
    DnBack,
    Back,
    UpBack,
}

impl Dir8 {
    pub fn relative(input: PlayerInput, facing: i32) -> Self {
        let up = input.dir & DIR_UP != 0;
        let down = input.dir & DIR_DOWN != 0;
        let left = input.dir & DIR_LEFT != 0;
        let right = input.dir & DIR_RIGHT != 0;

        let (fwd, back) = if facing >= 0 {
            (right, left)
        } else {
            (left, right)
        };

        match (up, down, fwd, back) {
            (true, _, true, _) => Self::UpFwd,
            (true, _, _, true) => Self::UpBack,
            (true, _, _, _) => Self::Up,
            (_, true, true, _) => Self::DnFwd,
            (_, true, _, true) => Self::DnBack,
            (_, true, _, _) => Self::Down,
            (_, _, true, _) => Self::Fwd,
            (_, _, _, true) => Self::Back,
            _ => Self::Neutral,
        }
    }

    pub fn is_forward(self) -> bool {
        matches!(self, Self::Fwd | Self::UpFwd | Self::DnFwd)
    }

    pub fn is_back(self) -> bool {
        matches!(self, Self::Back | Self::UpBack | Self::DnBack)
    }

    pub fn is_up(self) -> bool {
        matches!(self, Self::Up | Self::UpFwd | Self::UpBack)
    }

    pub fn is_down(self) -> bool {
        matches!(self, Self::Down | Self::DnFwd | Self::DnBack)
    }
}

impl fmt::Display for Dir8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Neutral => "N",
            Self::Up => "U",
            Self::UpFwd => "UF",
            Self::Fwd => "F",
            Self::DnFwd => "DF",
            Self::Down => "D",
            Self::DnBack => "DB",
            Self::Back => "B",
            Self::UpBack => "UB",
        };
        f.write_str(s)
    }
}

/// Maps a keyboard key name (as written in config files) to input bits.
pub fn key_to_bit(name: &str) -> Option<(u8, bool)> {
    let n = name.trim().to_ascii_lowercase();
    let bit = match n.as_str() {
        "up" => (DIR_UP, true),
        "down" => (DIR_DOWN, true),
        "left" => (DIR_LEFT, true),
        "right" => (DIR_RIGHT, true),
        "x" => (Buttons::X, false),
        "y" => (Buttons::Y, false),
        "z" => (Buttons::Z, false),
        "a" => (Buttons::A, false),
        "b" => (Buttons::B, false),
        "c" => (Buttons::C, false),
        "start" | "enter" | "return" => (Buttons::START, false),
        _ => return None,
    };
    Some(bit)
}
