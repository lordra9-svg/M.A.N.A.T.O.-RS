//! Screenpack fight-screen rendering (`docs/spec/render.md` §6).
//!
//! The fight screen is painted in two passes so the `envcolor` overlay can sit
//! between them: background elements (`bg*`, `mid`, `red`, `front`, `shift`,
//! `top`), then foreground elements (text, faces, win icons, combo counters,
//! round announcements).

use std::collections::BTreeMap;

use manato_core::{Vec2, GAME_HEIGHT, GAME_WIDTH};
use manato_formats::screenpack::{
    ActionDef, BarDef, ComboDef, Element, FaceDef, FightDef, Layer, NameDef, Screenpack, TextDef,
    TimeDef, WinIconDef,
};
use manato_sim::{RoundPhase, SimState, MAX_POWER};

use crate::atlas::{bank, SpriteAtlas};
use crate::draw::{Blend, Painter, TexRef};
use crate::screenfont::ScreenFont;
use crate::text::FontAtlas;

pub struct ScreenDeps<'a> {
    pub atlas: &'a SpriteAtlas,
    pub font: &'a FontAtlas,
    pub screen_fonts: &'a BTreeMap<u32, ScreenFont>,
}

pub type FightDeps<'a> = ScreenDeps<'a>;

/// Maps screenpack `localcoord` to the viewport (uniform, letterboxed; §10).
#[derive(Clone, Copy)]
pub struct ScreenMap {
    pub scale: f32,
    pub offset: Vec2,
}

impl ScreenMap {
    pub fn new(screenpack: &Screenpack) -> Self {
        let (w, h) = screenpack.localcoord;
        let (w, h) = if w > 0.0 && h > 0.0 {
            (w, h)
        } else {
            (320.0, 240.0)
        };
        let scale = (GAME_WIDTH / w).min(GAME_HEIGHT / h);
        Self {
            scale,
            offset: Vec2::new(
                (GAME_WIDTH - w * scale) * 0.5,
                (GAME_HEIGHT - h * scale) * 0.5,
            ),
        }
    }

    pub fn point(&self, local: Vec2) -> Vec2 {
        self.offset + local * self.scale
    }
}

struct SpriteFrame {
    sprite: crate::atlas::AtlasSprite,
    offset: Vec2,
    scale: Vec2,
    flip_h: bool,
    flip_v: bool,
    alpha: f32,
    blend: Blend,
}

fn blend_of(blend: manato_formats::air::BlendMode) -> Blend {
    match blend {
        manato_formats::air::BlendMode::Normal => Blend::Normal,
        manato_formats::air::BlendMode::Add => Blend::Add,
        manato_formats::air::BlendMode::Sub | manato_formats::air::BlendMode::SubAdd => Blend::Sub,
    }
}

/// Resolves the sprite and pose for a layer at the current frame.
fn layer_frame<'a>(
    deps: &'a FightDeps<'a>,
    layer: &Layer,
    fight: &'a FightDef,
    time: i32,
) -> Option<SpriteFrame> {
    let (bank_id, group, index, frame) = if let Some(number) = layer.anim {
        let animation = fight.animations.get(&number)?;
        let (frame_idx, _) = animation.frame_at(time)?;
        let frame = animation.frames.get(frame_idx)?;
        (bank::SCREENPACK, frame.group, frame.index, Some(frame))
    } else {
        let (group, index) = layer.sprite?;
        (bank::SCREENPACK, group, index, None)
    };
    let sprite = deps.atlas.get(bank_id, group, index)?;
    let (offset, scale, flip_h, flip_v, alpha, blend) = match frame {
        Some(frame) => (
            frame.offset,
            frame.scale,
            frame.flip_h,
            frame.flip_v,
            frame.alpha(),
            blend_of(frame.blend),
        ),
        None => (
            Vec2::ZERO,
            Vec2::new(1.0, 1.0),
            false,
            false,
            1.0,
            blend_of(layer.blend),
        ),
    };
    Some(SpriteFrame {
        sprite,
        offset,
        scale,
        flip_h,
        flip_v,
        alpha,
        blend,
    })
}

pub(crate) fn draw_layer(
    painter: &mut Painter,
    deps: &FightDeps,
    layer: &Layer,
    fight: &FightDef,
    map: &ScreenMap,
    origin: Vec2,
    time: i32,
    color: [f32; 4],
) {
    let Some(frame) = layer_frame(deps, layer, fight, time) else {
        return;
    };
    let dest_origin = map.point(origin + layer.offset + frame.offset);
    let w = frame.sprite.width * layer.scale.x * frame.scale.x.abs() * map.scale;
    let h = frame.sprite.height * layer.scale.y * frame.scale.y.abs() * map.scale;
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let mut color = color;
    color[3] *= frame.alpha;
    painter.quad(
        TexRef::Atlas(frame.sprite.page),
        frame.blend,
        [
            dest_origin.x,
            dest_origin.y,
            dest_origin.x + w,
            dest_origin.y + h,
        ],
        frame.sprite.uv,
        frame.flip_h,
        frame.flip_v,
        color,
    );
}

/// Draws a cropped fill layer. `ratio` in `0..=1`, anchored to `right` for P2.
#[allow(clippy::too_many_arguments)]
fn draw_fill(
    painter: &mut Painter,
    deps: &FightDeps,
    layer: &Layer,
    fight: &FightDef,
    map: &ScreenMap,
    origin: Vec2,
    time: i32,
    ratio: f32,
    right: bool,
    width: f32,
    color: [f32; 4],
) {
    let ratio = ratio.clamp(0.0, 1.0);
    if ratio <= 0.0 {
        return;
    }
    let Some(frame) = layer_frame(deps, layer, fight, time) else {
        return;
    };
    let dest_origin = map.point(origin + layer.offset + frame.offset);
    let full_w = frame.sprite.width * layer.scale.x * frame.scale.x.abs() * map.scale;
    let full_h = frame.sprite.height * layer.scale.y * frame.scale.y.abs() * map.scale;
    if full_w <= 0.0 || full_h <= 0.0 {
        return;
    }
    let fill_w = (width * ratio).clamp(1.0, GAME_WIDTH);
    let x = if right {
        dest_origin.x + full_w - fill_w
    } else {
        dest_origin.x
    };
    let scissor = [
        x.round().max(0.0) as u32,
        dest_origin.y.round().max(0.0) as u32,
        fill_w.round().max(1.0) as u32,
        full_h.round().max(1.0) as u32,
    ];
    painter.set_scissor(Some(scissor));
    let mut color = color;
    color[3] *= frame.alpha;
    painter.quad(
        TexRef::Atlas(frame.sprite.page),
        frame.blend,
        [
            dest_origin.x,
            dest_origin.y,
            dest_origin.x + full_w,
            dest_origin.y + full_h,
        ],
        frame.sprite.uv,
        frame.flip_h,
        frame.flip_v,
        color,
    );
    painter.set_scissor(None);
}

/// Draws a bit of fight-screen text, falling back to the built-in font.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_text(
    painter: &mut Painter,
    deps: &FightDeps,
    def: &TextDef,
    text: &str,
    map: &ScreenMap,
    origin: Vec2,
    align: i32,
    color: [f32; 4],
) {
    if text.is_empty() {
        return;
    }
    let pos = map.point(origin + def.offset);
    let scale = def.scale * map.scale;
    let alpha = def.color[3] as f32 / 255.0;
    let tint = [
        color[0] * def.color[0] as f32 / 255.0,
        color[1] * def.color[1] as f32 / 255.0,
        color[2] * def.color[2] as f32 / 255.0,
        color[3] * alpha,
    ];
    if let Some(font) = deps.screen_fonts.get(&def.font) {
        font.draw(painter, text, pos, scale, align, tint);
        return;
    }
    // Built-in fallback: 8px glyphs, anchored per `align`.
    let pixel_scale = (scale.y.max(scale.x) * 1.0).clamp(0.4, 4.0);
    let width = Painter::text_width(text, pixel_scale);
    let x = match align {
        1 => pos.x - width * 0.5,
        2 => pos.x - width,
        _ => pos.x,
    };
    painter.text(deps.font, text, x, pos.y, pixel_scale, tint);
}

fn replace_value(text: &str, value: &str, percent: Option<i32>) -> String {
    let mut out = text.replace("%d", value).replace("%i", value);
    if let Some(percent) = percent {
        out = out.replace("%f", &percent.to_string());
        out = out.replace("%p", &percent.to_string());
    }
    out.replace("%%", "%")
}

fn life_bar(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    side: usize,
    def: &BarDef,
    map: &ScreenMap,
    ratio: f32,
    mid: f32,
    value_text: String,
) {
    let time = sim.frame as i32;
    let right = side == 1;
    let origin = def.pos;
    let width = if def.range.x > 0.0 {
        def.range.x * map.scale
    } else {
        GAME_WIDTH * 0.5
    };
    let height = if def.range.y > 0.0 {
        def.range.y * map.scale
    } else {
        8.0 * map.scale
    };

    // Background layers.
    draw_layer(painter, deps, &def.bg0, fight, map, origin, time, [1.0; 4]);
    if ratio < 0.5 {
        draw_layer(painter, deps, &def.bg1, fight, map, origin, time, [1.0; 4]);
    }
    if ratio < 0.25 {
        draw_layer(painter, deps, &def.bg2, fight, map, origin, time, [1.0; 4]);
    }
    // Damage fill drains behind the live fill.
    draw_fill(
        painter, deps, &def.mid, fight, map, origin, time, mid, right, width, [1.0; 4],
    );
    draw_fill(
        painter, deps, &def.front, fight, map, origin, time, ratio, right, width, [1.0; 4],
    );
    draw_fill(
        painter, deps, &def.shift, fight, map, origin, time, ratio, right, width, [1.0; 4],
    );
    // Overlay and warning.
    draw_layer(painter, deps, &def.top, fight, map, origin, time, [1.0; 4]);
    let warn = def.warn_range;
    if (warn.x > 0.0 || warn.y > 0.0) && ratio <= (warn.x / 100.0) && ratio >= (warn.y / 100.0) {
        draw_layer(painter, deps, &def.warn, fight, map, origin, time, [1.0; 4]);
    }

    // Numeric value.
    if !def.value.text.is_empty() {
        let percent = (ratio * 100.0).round() as i32;
        let text = replace_value(&def.value.text, &value_text, Some(percent));
        draw_text(
            painter,
            deps,
            &def.value,
            &text,
            map,
            origin,
            def.value.align,
            [1.0; 4],
        );
    }
    let _ = height;
}

fn power_bar(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    side: usize,
    def: &BarDef,
    map: &ScreenMap,
    ratio: f32,
    mid: f32,
    power: f32,
) {
    let time = sim.frame as i32;
    let right = side == 1;
    let origin = def.pos;
    let width = if def.range.x > 0.0 {
        def.range.x * map.scale
    } else {
        GAME_WIDTH * 0.5
    };
    draw_layer(painter, deps, &def.bg0, fight, map, origin, time, [1.0; 4]);
    draw_fill(
        painter, deps, &def.mid, fight, map, origin, time, mid, right, width, [1.0; 4],
    );
    draw_fill(
        painter, deps, &def.front, fight, map, origin, time, ratio, right, width, [1.0; 4],
    );
    for (index, level) in def.level.iter().enumerate() {
        let threshold = (index as f32 + 1.0) / 3.0;
        if ratio >= threshold {
            draw_layer(painter, deps, level, fight, map, origin, time, [1.0; 4]);
        }
    }
    draw_layer(painter, deps, &def.top, fight, map, origin, time, [1.0; 4]);
    if !def.counter.text.is_empty() {
        let text = replace_value(
            &def.counter.text,
            &(power / 1000.0).floor().to_string(),
            None,
        );
        draw_text(
            painter,
            deps,
            &def.counter,
            &text,
            map,
            origin,
            def.counter.align,
            [1.0; 4],
        );
    }
    if !def.value.text.is_empty() {
        let percent = (ratio * 100.0).round() as i32;
        let text = replace_value(&def.value.text, &power.floor().to_string(), Some(percent));
        draw_text(
            painter,
            deps,
            &def.value,
            &text,
            map,
            origin,
            def.value.align,
            [1.0; 4],
        );
    }
}

fn face(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    side: usize,
    def: &FaceDef,
    map: &ScreenMap,
) {
    let time = sim.frame as i32;
    draw_layer(painter, deps, &def.bg, fight, map, def.pos, time, [1.0; 4]);
    draw_layer(painter, deps, &def.bg0, fight, map, def.pos, time, [1.0; 4]);
    draw_layer(painter, deps, &def.bg1, fight, map, def.pos, time, [1.0; 4]);
    draw_layer(painter, deps, &def.bg2, fight, map, def.pos, time, [1.0; 4]);

    if let Some((group, index)) = def.face_spr {
        if let Some(sprite) = deps
            .atlas
            .get(bank::CHARACTER_BASE + side as u8, group, index)
        {
            let origin = map.point(def.pos);
            let w = sprite.width * map.scale;
            let h = sprite.height * map.scale;
            painter.quad(
                TexRef::Atlas(sprite.page),
                Blend::Normal,
                [origin.x, origin.y, origin.x + w, origin.y + h],
                sprite.uv,
                side == 1,
                false,
                [1.0; 4],
            );
        }
    }
    draw_layer(painter, deps, &def.top, fight, map, def.pos, time, [1.0; 4]);
    if sim.fighters[side].life <= 0 {
        draw_layer(painter, deps, &def.ko, fight, map, def.pos, time, [1.0; 4]);
    }
}

fn name(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    side: usize,
    def: &NameDef,
    map: &ScreenMap,
) {
    let time = sim.frame as i32;
    draw_layer(painter, deps, &def.bg, fight, map, def.pos, time, [1.0; 4]);
    let text = if def.name.text.is_empty() {
        sim.assets.characters[side].display_name().to_string()
    } else {
        def.name.text.clone()
    };
    draw_text(
        painter,
        deps,
        &def.name,
        &text,
        map,
        def.pos,
        def.name.align,
        [1.0; 4],
    );
    draw_layer(painter, deps, &def.top, fight, map, def.pos, time, [1.0; 4]);
}

fn time(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    def: &TimeDef,
    map: &ScreenMap,
) {
    if sim.round.phase == RoundPhase::Over {
        return;
    }
    let time = sim.frame as i32;
    draw_layer(painter, deps, &def.bg, fight, map, def.pos, time, [1.0; 4]);
    let seconds = sim.round.seconds();
    let text = if def.counter.text.is_empty() {
        format!("{seconds:02}")
    } else {
        replace_value(&def.counter.text, &seconds.to_string(), None)
    };
    draw_text(
        painter,
        deps,
        &def.counter,
        &text,
        map,
        def.pos,
        def.counter.align,
        [1.0; 4],
    );
    draw_layer(painter, deps, &def.top, fight, map, def.pos, time, [1.0; 4]);
}

fn win_icons(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    side: usize,
    def: &WinIconDef,
    map: &ScreenMap,
) {
    let time = sim.frame as i32;
    let wins = &sim.win_history[side];
    let limit = if def.use_icon_upto > 0 {
        wins.len().min(def.use_icon_upto as usize)
    } else {
        wins.len()
    };
    let step = Vec2::new(
        if side == 1 {
            -def.icon_offset.x
        } else {
            def.icon_offset.x
        },
        def.icon_offset.y,
    );
    for (index, record) in wins.iter().take(limit).enumerate() {
        let origin = def.pos + step * index as f32;
        draw_layer(painter, deps, &def.bg0, fight, map, origin, time, [1.0; 4]);
        let icon = if record.time_over {
            &def.time
        } else {
            &def.normal
        };
        draw_layer(painter, deps, icon, fight, map, origin, time, [1.0; 4]);
        if record.perfect {
            draw_layer(
                painter,
                deps,
                &def.perfect,
                fight,
                map,
                origin,
                time,
                [1.0; 4],
            );
        } else if record.clutch {
            draw_layer(
                painter,
                deps,
                &def.clutch,
                fight,
                map,
                origin,
                time,
                [1.0; 4],
            );
        }
        draw_layer(painter, deps, &def.top, fight, map, origin, time, [1.0; 4]);
    }
    if !def.counter.text.is_empty() {
        let text = replace_value(&def.counter.text, &wins.len().to_string(), None);
        draw_text(
            painter,
            deps,
            &def.counter,
            &text,
            map,
            def.pos,
            def.counter.align,
            [1.0; 4],
        );
    }
}

fn combo(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    side: usize,
    def: &ComboDef,
    map: &ScreenMap,
) {
    let state = &sim.combo[side];
    if state.shown_hits < 2 || state.timer <= 0 {
        return;
    }
    let time = sim.frame as i32;
    // Slide in from `start.x`, easing back out as the display timer expires.
    let progress = (state.timer as f32 / manato_sim::COMBO_DISPLAY_TICKS as f32).clamp(0.0, 1.0);
    let origin = def.pos + Vec2::new(def.start_x * progress, 0.0);
    draw_layer(painter, deps, &def.bg, fight, map, origin, time, [1.0; 4]);
    let hits = state.shown_hits.to_string();
    let counter = if def.counter.text.is_empty() {
        hits.clone()
    } else {
        replace_value(&def.counter.text, &hits, None)
    };
    draw_text(
        painter,
        deps,
        &def.counter,
        &counter,
        map,
        origin,
        def.counter.align,
        [1.0; 4],
    );
    if !def.text.text.is_empty() {
        let label = replace_value(&def.text.text, &hits, None);
        draw_text(
            painter,
            deps,
            &def.text,
            &label,
            map,
            origin,
            def.text.align,
            [1.0; 4],
        );
    }
    draw_layer(painter, deps, &def.top, fight, map, origin, time, [1.0; 4]);
}

/// Draws `[Action]` banners for one team (`docs/spec/render.md` §6.10).
fn actions(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    side: usize,
    def: &ActionDef,
    map: &ScreenMap,
) {
    let time = sim.frame as i32;
    for (index, message) in sim.actions[side].iter().enumerate() {
        let progress = (message.time as f32 / message.total.max(1) as f32).clamp(0.0, 1.0);
        let origin = def.pos + def.spacing * index as f32 + Vec2::new(def.start_x * progress, 0.0);
        draw_layer(painter, deps, &def.bg, fight, map, origin, time, [1.0; 4]);
        draw_text(
            painter,
            deps,
            &def.text,
            &message.text,
            map,
            origin,
            def.text.align,
            [1.0; 4],
        );
        draw_layer(
            painter, deps, &def.front, fight, map, origin, time, [1.0; 4],
        );
    }
}

fn round_announcement(
    painter: &mut Painter,
    deps: &FightDeps,
    fight: &FightDef,
    sim: &SimState,
    map: &ScreenMap,
) {
    let round = &fight.round;
    let name = match sim.round.phase {
        RoundPhase::Intro => {
            let number = format!("round{}", sim.round.number);
            if round.elements.contains_key(&number) {
                Some(number)
            } else if sim.round.number == sim.round.rounds_to_win * 2 - 1
                && round.elements.contains_key("round.final")
            {
                Some("round.final".to_string())
            } else if round.elements.contains_key("round.default") {
                Some("round.default".to_string())
            } else if round.elements.contains_key("round.single") {
                Some("round.single".to_string())
            } else {
                None
            }
        }
        RoundPhase::Ko => {
            if sim.round.time_over {
                Some("to".to_string())
            } else if sim.round.winner.is_none() {
                Some("dko".to_string())
            } else {
                Some("win".to_string())
            }
        }
        _ => None,
    };
    let Some(name) = name else {
        return;
    };
    let Some(element) = round.elements.get(&name) else {
        return;
    };
    let mut element = element.clone();
    if !element.text.is_empty() {
        element.text = element.text.replace("%d", &sim.round.number.to_string());
    }
    draw_element(painter, deps, &element, fight, map, round.pos);
}

pub(crate) fn draw_element(
    painter: &mut Painter,
    deps: &FightDeps,
    element: &Element,
    fight: &FightDef,
    map: &ScreenMap,
    parent: Vec2,
) {
    let origin = parent + element.pos;
    let text_origin = origin + element.offset;
    if let Some((group, index)) = element.sprite {
        if let Some(sprite) = deps.atlas.get(bank::SCREENPACK, group, index) {
            let pos = map.point(origin);
            let w = sprite.width * element.scale.x * map.scale;
            let h = sprite.height * element.scale.y * map.scale;
            painter.quad(
                TexRef::Atlas(sprite.page),
                Blend::Normal,
                [pos.x, pos.y, pos.x + w, pos.y + h],
                sprite.uv,
                false,
                false,
                [1.0; 4],
            );
        }
    } else if let Some(number) = element.anim {
        if let Some(animation) = fight.animations.get(&number) {
            let (frame_idx, _) = animation.frame_at(0).unwrap_or((0, 0));
            if let Some(frame) = animation.frames.get(frame_idx) {
                if let Some(sprite) = deps.atlas.get(bank::SCREENPACK, frame.group, frame.index) {
                    let pos = map.point(origin + frame.offset);
                    let w = sprite.width * element.scale.x * frame.scale.x * map.scale;
                    let h = sprite.height * element.scale.y * frame.scale.y * map.scale;
                    painter.quad(
                        TexRef::Atlas(sprite.page),
                        Blend::Normal,
                        [pos.x, pos.y, pos.x + w, pos.y + h],
                        sprite.uv,
                        frame.flip_h,
                        frame.flip_v,
                        [1.0, 1.0, 1.0, frame.alpha()],
                    );
                }
            }
        }
    }
    if !element.text.is_empty() {
        let text_def = TextDef {
            font: element.font,
            text: element.text.clone(),
            offset: Vec2::ZERO,
            scale: element.scale,
            align: element.align,
            color: element.color,
        };
        draw_text(
            painter,
            deps,
            &text_def,
            &element.text,
            map,
            text_origin,
            element.align,
            [1.0; 4],
        );
    }
}

/// Paints the fight-screen background elements (spec paint order pass 10).
pub fn draw_background(
    painter: &mut Painter,
    sim: &SimState,
    deps: &FightDeps,
    screenpack: &Screenpack,
) {
    let map = ScreenMap::new(screenpack);
    let fight = &screenpack.fight;
    for side in 0..2 {
        let fighter = &sim.fighters[side];
        let life_ratio = (fighter.life as f32 / fighter.life_max.max(1) as f32).clamp(0.0, 1.0);
        let power_ratio = (fighter.power / MAX_POWER).clamp(0.0, 1.0);
        let bar = &sim.bars[side];
        life_bar(
            painter,
            deps,
            fight,
            sim,
            side,
            &fight.life[side],
            &map,
            life_ratio,
            bar.mid_life.clamp(0.0, 1.0),
            fighter.life.to_string(),
        );
        power_bar(
            painter,
            deps,
            fight,
            sim,
            side,
            &fight.power[side],
            &map,
            power_ratio,
            bar.mid_power.clamp(0.0, 1.0),
            fighter.power,
        );
        face(painter, deps, fight, sim, side, &fight.face[side], &map);
        name(painter, deps, fight, sim, side, &fight.name_def[side], &map);
    }
    time(painter, deps, fight, sim, &fight.time, &map);
}

/// Paints the fight-screen foreground elements (spec paint order pass 12).
pub fn draw_foreground(
    painter: &mut Painter,
    sim: &SimState,
    deps: &FightDeps,
    screenpack: &Screenpack,
) {
    let map = ScreenMap::new(screenpack);
    let fight = &screenpack.fight;
    for side in 0..2 {
        win_icons(painter, deps, fight, sim, side, &fight.win_icon[side], &map);
        combo(painter, deps, fight, sim, side, &fight.combo[side], &map);
        actions(painter, deps, fight, sim, side, &fight.action[side], &map);
    }
    round_announcement(painter, deps, fight, sim, &map);
}

#[cfg(test)]
mod tests {
    use super::*;
    use manato_formats::screenpack::Screenpack;

    fn map(localcoord: (f32, f32)) -> ScreenMap {
        let mut pack = Screenpack::default();
        pack.localcoord = localcoord;
        ScreenMap::new(&pack)
    }

    #[test]
    fn maps_localcoord_to_the_viewport_with_letterboxing() {
        let native = map((320.0, 240.0));
        assert!((native.scale - 1.0).abs() < f32::EPSILON);
        assert_eq!(native.offset, Vec2::new(0.0, 0.0));
        assert_eq!(
            native.point(Vec2::new(160.0, 120.0)),
            Vec2::new(160.0, 120.0)
        );

        let double = map((640.0, 480.0));
        assert!((double.scale - 0.5).abs() < f32::EPSILON);
        assert_eq!(double.offset, Vec2::new(0.0, 0.0));

        // A wide localcoord letterboxes vertically but stays centred.
        let wide = map((640.0, 240.0));
        assert!((wide.scale - 0.5).abs() < f32::EPSILON);
        assert_eq!(wide.offset, Vec2::new(0.0, 60.0));
        assert_eq!(wide.point(Vec2::ZERO), Vec2::new(0.0, 60.0));

        // Zero/invalid localcoord falls back to 320x240.
        let invalid = map((0.0, 0.0));
        assert!((invalid.scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fill_values_are_replaced() {
        assert_eq!(replace_value("%d / %f", "42", Some(60)), "42 / 60");
        assert_eq!(replace_value("%p%%", "7", Some(70)), "70%");
        assert_eq!(replace_value("plain", "1", None), "plain");
    }
}
