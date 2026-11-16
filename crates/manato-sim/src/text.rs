//! Runtime text objects created by the `Text` controller (`docs/spec/text.md`).

use manato_core::Vec2;

/// A single on-screen text sprite.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextObject {
    pub owner: usize,
    pub id: i32,
    pub text: String,
    /// Stage-space position (already offset by the spawn `pos`).
    pub pos: Vec2,
    /// Depth position for `[Scaling]` perspective.
    pub z: f32,
    pub vel: Vec2,
    pub accel: Vec2,
    pub friction: Vec2,
    /// Ticks since creation; drives `textdelay` typing.
    pub time: i32,
    /// Remaining lifetime in ticks; `0` removes the text on the next prune.
    pub remove_time: i32,
    /// Draw order: negative behind fighters, `0`/positive in front.
    pub layer_no: i32,
    /// Screenpack/fight-screen font index (`-1` = built-in).
    pub font: i32,
    /// Font table: `f` (fight.def), `m` (system.def), any other value uses
    /// the fight-screen table as the engine's closest available bank.
    pub font_bank: char,
    pub align: i32,
    pub scale: Vec2,
    pub color: [u8; 4],
    pub spacing: Vec2,
    /// Ticks per revealed character (`0` = show the whole string).
    pub text_delay: f32,
    pub angle: f32,
    pub x_shear: f32,
}

impl TextObject {
    pub fn new(owner: usize, text: String, pos: Vec2) -> Self {
        Self {
            owner,
            id: 0,
            text,
            pos,
            z: 0.0,
            vel: Vec2::ZERO,
            accel: Vec2::ZERO,
            friction: Vec2::new(1.0, 1.0),
            time: 0,
            remove_time: 1,
            layer_no: 0,
            font: -1,
            font_bank: 'f',
            align: 1,
            scale: Vec2::new(1.0, 1.0),
            color: [255, 255, 255, 255],
            spacing: Vec2::ZERO,
            text_delay: 0.0,
            angle: 0.0,
            x_shear: 0.0,
        }
    }

    /// Advances motion and the lifetime countdown by one tick.
    pub fn tick(&mut self) {
        self.time += 1;
        self.vel.x = self.vel.x * self.friction.x + self.accel.x;
        self.vel.y = self.vel.y * self.friction.y + self.accel.y;
        self.pos.x += self.vel.x;
        self.pos.y += self.vel.y;
        if self.remove_time > 0 {
            self.remove_time -= 1;
        }
    }

    pub fn expired(&self) -> bool {
        self.remove_time == 0
    }

    /// The visible prefix of the string after `textdelay` typing.
    pub fn visible_text(&self) -> &str {
        if self.text_delay <= 0.0 {
            return &self.text;
        }
        let count = ((self.time as f32 + self.text_delay) / self.text_delay) as usize;
        let end = count.min(self.text.chars().count());
        match self.text.char_indices().nth(end) {
            Some((byte, _)) => &self.text[..byte],
            None => &self.text,
        }
    }
}
