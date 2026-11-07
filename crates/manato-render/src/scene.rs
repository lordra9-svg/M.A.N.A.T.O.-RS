//! Turns a [`SimState`] snapshot into draw commands.

use manato_core::{Vec2, GAME_HEIGHT, GAME_WIDTH};
use manato_formats::air::{AnimFrame, BlendMode};
use manato_formats::{Animation, Character, Sprite, Stage};
use manato_sim::{Effect, EffectBlend, EffectSource, RoundPhase, SimState, MAX_POWER};

use crate::atlas::{bank, SpriteAtlas};
use crate::draw::{Blend, Painter, TexRef};
use crate::text::{fit_chars, FontAtlas, GLYPH_H};

pub struct SceneDeps<'a> {
    pub atlas: &'a SpriteAtlas,
    pub font: &'a FontAtlas,
}

#[derive(Clone, Copy)]
struct Pose {
    offset: Vec2,
    flip_h: bool,
    flip_v: bool,
    blend: Blend,
    scale: Vec2,
    alpha: f32,
    angle: f32,
}

impl Pose {
    fn from_frame(frame: &AnimFrame) -> Self {
        Self {
            offset: frame.offset,
            flip_h: frame.flip_h,
            flip_v: frame.flip_v,
            blend: blend_of(frame.blend),
            scale: frame.scale,
            alpha: frame.alpha(),
            angle: frame.angle,
        }
    }
}

fn blend_of(blend: BlendMode) -> Blend {
    match blend {
        BlendMode::Normal => Blend::Normal,
        BlendMode::Add => Blend::Add,
        BlendMode::Sub | BlendMode::SubAdd => Blend::Sub,
    }
}

pub fn build(painter: &mut Painter, sim: &SimState, deps: &SceneDeps) {
    painter.clear();
    let stage = &sim.assets.stage;

    for layer in stage.render_layers().filter(|l| l.layer_no < 0) {
        draw_bg_layer(painter, deps.atlas, sim, stage, layer);
    }
    for layer in stage.render_layers().filter(|l| l.layer_no == 0) {
        draw_bg_layer(painter, deps.atlas, sim, stage, layer);
    }

    draw_shadows(painter, sim);

    // Effects that are explicitly behind characters.
    let mut behind: Vec<&Effect> = sim.effects.iter().filter(|e| e.behind).collect();
    behind.sort_by_key(|e| e.sprite_priority);
    for effect in behind {
        draw_effect(painter, deps.atlas, sim, effect);
    }

    let mut order: Vec<usize> = (0..sim.fighters.len()).collect();
    order.sort_by_key(|&i| sim.fighters[i].sprite_priority);
    for who in order {
        draw_fighter(painter, deps.atlas, sim, who);
    }

    draw_projectiles(painter, deps.atlas, sim);

    for layer in stage.render_layers().filter(|l| l.layer_no > 0) {
        draw_bg_layer(painter, deps.atlas, sim, stage, layer);
    }

    let mut front: Vec<&Effect> = sim.effects.iter().filter(|e| !e.behind).collect();
    front.sort_by_key(|e| e.sprite_priority);
    for effect in front {
        draw_effect(painter, deps.atlas, sim, effect);
    }

    if let Some(env_color) = sim.env_color {
        painter.rect(
            Blend::Normal,
            0.0,
            0.0,
            GAME_WIDTH,
            GAME_HEIGHT,
            [
                env_color.color[0].clamp(0.0, 1.0),
                env_color.color[1].clamp(0.0, 1.0),
                env_color.color[2].clamp(0.0, 1.0),
                0.35,
            ],
        );
    }

    draw_hud(painter, deps.font, sim);
}

fn draw_bg_layer(
    painter: &mut Painter,
    atlas: &SpriteAtlas,
    sim: &SimState,
    stage: &Stage,
    layer: &manato_formats::stage::BgLayer,
) {
    if !layer.visible || !layer.enabled {
        return;
    }
    let camera = &sim.camera;
    let time = sim.bg_time;
    let sin_offset = |motion: [f32; 3]| {
        if motion[1].abs() > f32::EPSILON {
            motion[0] * (std::f32::consts::TAU * time / motion[1] + motion[2].to_radians()).sin()
        } else {
            0.0
        }
    };
    let cos_offset = |motion: [f32; 3]| {
        if motion[1].abs() > f32::EPSILON {
            motion[0] * (std::f32::consts::TAU * time / motion[1] + motion[2].to_radians()).cos()
        } else {
            0.0
        }
    };
    let motion = Vec2::new(
        layer.velocity.x * time + sin_offset(layer.sin_x) + cos_offset(layer.cos_x),
        layer.velocity.y * time + sin_offset(layer.sin_y) + cos_offset(layer.cos_y),
    );
    let base = Vec2::new(
        layer.start.x - camera.x * layer.delta.x + camera.shake.x + motion.x,
        layer.start.y - camera.y * layer.delta.y + camera.shake.y + motion.y,
    );

    let (sprite, pose, anim) = if let Some(action) = layer.action {
        if let Some(anim) = stage.animations.get(&action) {
            let frame_idx = anim
                .frame_at(sim.bg_time as i32)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let frame = anim.frames.get(frame_idx);
            let (group, index, pose) = match frame {
                Some(f) => (f.group, f.index, Pose::from_frame(f)),
                None => return,
            };
            let Some(sprite) = stage.sprites.get(group, index) else {
                return;
            };
            (sprite, pose, Some(anim))
        } else {
            return;
        }
    } else if let Some((group, index)) = layer.sprite {
        let Some(sprite) = stage.sprites.get(group, index) else {
            return;
        };
        (
            sprite,
            Pose {
                offset: Vec2::ZERO,
                flip_h: false,
                flip_v: false,
                blend: blend_of(layer.trans),
                scale: Vec2::new(1.0, 1.0),
                alpha: 1.0,
                angle: 0.0,
            },
            None,
        )
    } else {
        return;
    };
    let _ = anim;

    let blend = blend_of(layer.trans);
    let (w, h) = (
        sprite.width as f32 * layer.xscale,
        sprite.height as f32 * layer.yscale,
    );
    let base_left = base.x
        + if pose.flip_h {
            pose.offset.x
                - (sprite.width as f32 * layer.xscale - sprite.axis_x as f32 * layer.xscale)
        } else {
            pose.offset.x - sprite.axis_x as f32 * layer.xscale
        };
    let base_top = base.y
        + if pose.flip_v {
            pose.offset.y
                - (sprite.height as f32 * layer.yscale - sprite.axis_y as f32 * layer.yscale)
        } else {
            pose.offset.y - sprite.axis_y as f32 * layer.yscale
        };
    let step_x = w + layer.tile_spacing.0 as f32;
    let step_y = h + layer.tile_spacing.1 as f32;
    if step_x <= 0.0 || step_y <= 0.0 {
        return;
    }
    let range = |tiled: bool, base: f32, step: f32, span: f32| -> Vec<f32> {
        if !tiled {
            return vec![base];
        }
        let mut out = Vec::new();
        let start = ((0.0 - base) / step).floor() as i32 - 1;
        let end = ((span - base) / step).ceil() as i32 + 1;
        for i in start..=end {
            out.push(base + i as f32 * step);
        }
        out
    };
    let xs = range(layer.tile.0 != 0, base_left, step_x, GAME_WIDTH);
    let ys = range(layer.tile.1 != 0, base_top, step_y, GAME_HEIGHT);
    for y in ys.iter().copied() {
        for x in xs.iter().copied() {
            draw_sprite_at(
                painter,
                atlas,
                bank::STAGE,
                sprite,
                [x, y],
                pose.flip_h,
                pose.flip_v,
                blend,
                [1.0, 1.0, 1.0, pose.alpha],
                layer.xscale * pose.scale.x,
                layer.yscale * pose.scale.y,
                pose.angle,
            );
        }
    }
}

fn draw_projectiles(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState) {
    for projectile in &sim.projectiles {
        let char = &sim.assets.characters[sim.fighters[projectile.owner].char_id];
        let Some(anim) = char.animation(projectile.anim) else {
            continue;
        };
        let Some((frame_idx, _)) = anim.frame_at(projectile.time) else {
            continue;
        };
        let Some(frame) = anim.frames.get(frame_idx) else {
            continue;
        };
        let Some(sprite) = char.sprites.get(frame.group, frame.index) else {
            continue;
        };
        let pose = Pose::from_frame(frame);
        draw_sprite(
            painter,
            atlas,
            bank::CHARACTER_BASE + projectile.owner as u8,
            sprite,
            sim.screen_pos(projectile.pos),
            pose.offset,
            projectile.facing,
            pose.scale,
            pose.flip_h,
            pose.flip_v,
            pose.blend,
            [1.0, 1.0, 1.0, pose.alpha],
            pose.angle,
        );
    }
}

fn draw_shadows(painter: &mut Painter, sim: &SimState) {
    for fighter in &sim.fighters {
        if fighter.owner.is_some()
            || !fighter.active
            || fighter.assert_flags & manato_sim::fighter::ASSERT_INVISIBLE != 0
            || fighter.assert_flags & manato_sim::fighter::ASSERT_NOSHADOW != 0
        {
            continue;
        }
        let ground = sim.screen_pos(Vec2::new(fighter.pos.x, 0.0));
        let height = (-fighter.pos.y).max(0.0);
        let shadow = &sim.assets.stage.shadow;
        let base_alpha = (shadow.intensity as f32 / 255.0).clamp(0.0, 1.0) * 0.6;
        let alpha = (base_alpha - height / 400.0).clamp(0.05, base_alpha.max(0.05));
        let w = (fighter.width_back + fighter.width_front) * 1.35;
        let h = 7.0;
        painter.quad(
            TexRef::Shadow,
            Blend::Normal,
            [
                ground.x - w * 0.5,
                ground.y - h * 0.5,
                ground.x + w * 0.5,
                ground.y + h * 0.5,
            ],
            [0.0, 0.0, 1.0, 1.0],
            false,
            false,
            [
                shadow.color[0] as f32 / 255.0,
                shadow.color[1] as f32 / 255.0,
                shadow.color[2] as f32 / 255.0,
                alpha,
            ],
        );
    }
}

fn draw_sprite(
    painter: &mut Painter,
    atlas: &SpriteAtlas,
    bank: u8,
    sprite: &Sprite,
    anchor: Vec2,
    offset: Vec2,
    facing: i32,
    scale: Vec2,
    flip_h: bool,
    flip_v: bool,
    blend: Blend,
    color: [f32; 4],
    angle: f32,
) {
    let Some(atlas_sprite) = atlas.get(bank, sprite.group, sprite.index) else {
        return;
    };
    let w = sprite.width as f32 * scale.x;
    let h = sprite.height as f32 * scale.y;
    let axis_x = sprite.axis_x as f32 * scale.x;
    let axis_y = sprite.axis_y as f32 * scale.y;
    let offset_x = offset.x * scale.x;
    let offset_y = offset.y * scale.y;
    // AIR H/V flags mirror around the sprite axis and already negate the
    // stored offset. Facing mirrors the whole character around the anchor.
    let local_left = if flip_h {
        offset_x - (w - axis_x)
    } else {
        offset_x - axis_x
    };
    let local_top = if flip_v {
        offset_y - (h - axis_y)
    } else {
        offset_y - axis_y
    };
    let facing_flip = facing < 0;
    let x0 = if facing_flip {
        anchor.x - local_left - w
    } else {
        anchor.x + local_left
    };
    let y0 = anchor.y + local_top;
    let flipped = facing_flip ^ flip_h;
    draw_atlas_quad(
        painter,
        atlas_sprite.page,
        atlas_sprite.uv,
        [x0, y0, x0 + w, y0 + h],
        flipped,
        flip_v,
        blend,
        color,
        angle,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_sprite_at(
    painter: &mut Painter,
    atlas: &SpriteAtlas,
    bank: u8,
    sprite: &Sprite,
    top_left: [f32; 2],
    flip_h: bool,
    flip_v: bool,
    blend: Blend,
    color: [f32; 4],
    scale_x: f32,
    scale_y: f32,
    angle: f32,
) {
    let Some(atlas_sprite) = atlas.get(bank, sprite.group, sprite.index) else {
        return;
    };
    let w = sprite.width as f32 * scale_x;
    let h = sprite.height as f32 * scale_y;
    draw_atlas_quad(
        painter,
        atlas_sprite.page,
        atlas_sprite.uv,
        [top_left[0], top_left[1], top_left[0] + w, top_left[1] + h],
        flip_h,
        flip_v,
        blend,
        color,
        angle,
    );
}

fn draw_atlas_quad(
    painter: &mut Painter,
    page: u32,
    uv: [f32; 4],
    dest: [f32; 4],
    flip_h: bool,
    flip_v: bool,
    blend: Blend,
    color: [f32; 4],
    angle: f32,
) {
    painter.quad_rotated(
        TexRef::Atlas(page),
        blend,
        dest,
        uv,
        flip_h,
        flip_v,
        color,
        angle,
    );
}

fn draw_fighter(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState, who: usize) {
    let fighter = &sim.fighters[who];
    if !fighter.active || fighter.assert_flags & manato_sim::fighter::ASSERT_INVISIBLE != 0 {
        return;
    }
    let character: &Character = &sim.assets.characters[fighter.char_id];
    let Some(frame) = fighter.current_frame(character) else {
        return;
    };
    let Some(sprite) = character.sprites.get(frame.group, frame.index) else {
        return;
    };
    let mut anchor = sim.screen_pos(fighter.pos);
    anchor.x += character.constants.get_or("draw.offset.x", 0.0) + fighter.draw_offset.x;
    anchor.y += character.constants.get_or("draw.offset.y", 0.0) + fighter.draw_offset.y;
    let base_scale = Vec2::new(
        character.constants.get_or("size.xscale", 1.0),
        character.constants.get_or("size.yscale", 1.0),
    );
    let pose = Pose::from_frame(frame);
    let scale = Vec2::new(base_scale.x * pose.scale.x, base_scale.y * pose.scale.y);

    for ghost in &fighter.after_images {
        let Some(anim) = character.animation(ghost.anim_no) else {
            continue;
        };
        let Some(frame) = anim
            .frames
            .get(ghost.anim_frame.min(anim.frames.len().saturating_sub(1)))
        else {
            continue;
        };
        let Some(sprite) = character.sprites.get(frame.group, frame.index) else {
            continue;
        };
        let ghost_pose = Pose::from_frame(frame);
        draw_sprite(
            painter,
            atlas,
            bank::CHARACTER_BASE + fighter.char_id as u8,
            sprite,
            sim.screen_pos(ghost.pos),
            ghost_pose.offset,
            ghost.facing,
            Vec2::new(
                base_scale.x * ghost_pose.scale.x,
                base_scale.y * ghost_pose.scale.y,
            ),
            ghost_pose.flip_h,
            ghost_pose.flip_v,
            if ghost.add { Blend::Add } else { Blend::Normal },
            [1.0, 1.0, 1.0, ghost.alpha],
            ghost_pose.angle,
        );
    }

    let tint = [
        (fighter.pal_mul[0] + fighter.pal_add[0]).clamp(0.0, 1.0),
        (fighter.pal_mul[1] + fighter.pal_add[1]).clamp(0.0, 1.0),
        (fighter.pal_mul[2] + fighter.pal_add[2]).clamp(0.0, 1.0),
        (pose.alpha * fighter.pal_alpha * fighter.trans_alpha).clamp(0.0, 1.0),
    ];
    draw_sprite(
        painter,
        atlas,
        bank::CHARACTER_BASE + fighter.char_id as u8,
        sprite,
        anchor,
        pose.offset,
        fighter.facing,
        scale,
        pose.flip_h,
        pose.flip_v,
        pose.blend,
        tint,
        pose.angle,
    );
}

fn effect_animation<'a>(sim: &'a SimState, effect: &Effect) -> Option<(&'a Animation, u8)> {
    match effect.source {
        EffectSource::FightFx => Some((
            sim.assets.effects.animations.get(&effect.anim)?,
            bank::EFFECTS,
        )),
        EffectSource::Character(i) => Some((
            sim.assets.characters.get(i)?.animations.get(&effect.anim)?,
            bank::CHARACTER_BASE + i as u8,
        )),
        EffectSource::Stage => Some((sim.assets.stage.animations.get(&effect.anim)?, bank::STAGE)),
    }
}

fn effect_sprite<'a>(
    sim: &'a SimState,
    effect: &Effect,
    group: i32,
    index: i32,
) -> Option<&'a Sprite> {
    match effect.source {
        EffectSource::FightFx => sim.assets.effects.sprites.get(group, index),
        EffectSource::Character(i) => sim.assets.characters.get(i)?.sprites.get(group, index),
        EffectSource::Stage => sim.assets.stage.sprites.get(group, index),
    }
}

fn draw_effect(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState, effect: &Effect) {
    let Some((anim, bank)) = effect_animation(sim, effect) else {
        return;
    };
    let Some((frame_idx, _)) = anim.frame_at(effect.time) else {
        return;
    };
    let Some(frame) = anim.frames.get(frame_idx) else {
        return;
    };
    let Some(sprite) = effect_sprite(sim, effect, frame.group, frame.index) else {
        return;
    };
    let anchor = sim.screen_pos(effect.pos);
    let blend = match effect.blend {
        EffectBlend::Normal => Blend::Normal,
        EffectBlend::Add => Blend::Add,
        EffectBlend::Sub => Blend::Sub,
    };
    let pose = Pose::from_frame(frame);
    draw_sprite(
        painter,
        atlas,
        bank,
        sprite,
        anchor,
        Vec2::new(frame.offset.x, frame.offset.y),
        effect.facing,
        Vec2::new(effect.scale.x * pose.scale.x, effect.scale.y * pose.scale.y),
        frame.flip_h ^ effect.flip_h,
        frame.flip_v ^ effect.flip_v,
        blend,
        [1.0, 1.0, 1.0, (effect.alpha * pose.alpha).clamp(0.0, 1.0)],
        pose.angle,
    );
}

fn draw_hud(painter: &mut Painter, font: &FontAtlas, sim: &SimState) {
    let bar_w = 148.0;
    let bar_h = 10.0;
    let y = 8.0;
    let p1_x = 6.0;
    let p2_x = GAME_WIDTH - 6.0 - bar_w;

    for (who, x) in [(0usize, p1_x), (1usize, p2_x)] {
        let fighter = &sim.fighters[who];
        let ratio = (fighter.life as f32 / fighter.life_max.max(1) as f32).clamp(0.0, 1.0);
        painter.rect(
            Blend::Normal,
            x - 1.0,
            y - 1.0,
            bar_w + 2.0,
            bar_h + 2.0,
            [0.05, 0.05, 0.07, 0.9],
        );
        painter.rect(Blend::Normal, x, y, bar_w, bar_h, [0.22, 0.06, 0.06, 1.0]);
        let fill_w = (bar_w * ratio).round();
        let color = if ratio < 0.25 {
            [0.95, 0.32, 0.22, 1.0]
        } else {
            [0.98, 0.82, 0.2, 1.0]
        };
        if who == 0 {
            painter.rect(Blend::Normal, x, y, fill_w, bar_h, color);
        } else {
            painter.rect(Blend::Normal, x + bar_w - fill_w, y, fill_w, bar_h, color);
        }

        // Match wins.
        for win in 0..sim.round.wins[who].min(3) {
            let pip_x = if who == 0 {
                x + bar_w - 8.0 - win as f32 * 9.0
            } else {
                x + 8.0 + win as f32 * 9.0
            };
            painter.rect(
                Blend::Normal,
                pip_x,
                y + bar_h + 3.0,
                6.0,
                6.0,
                [0.95, 0.85, 0.3, 1.0],
            );
        }

        // Name, clipped to the bar width.
        let name = sim.assets.characters[who].display_name();
        let limit = fit_chars(bar_w, 1.0);
        let short: String = name.chars().take(limit).collect();
        painter.text(
            font,
            &short,
            x + 1.0,
            y + bar_h + 4.0,
            1.0,
            [0.92, 0.92, 0.96, 1.0],
        );
    }

    // Timer.
    let seconds = sim.round.seconds();
    painter.rect(
        Blend::Normal,
        GAME_WIDTH * 0.5 - 18.0,
        y - 1.0,
        36.0,
        18.0,
        [0.05, 0.05, 0.07, 0.85],
    );
    painter.text_centered(
        font,
        &format!("{seconds:02}"),
        GAME_WIDTH * 0.5,
        y + 1.0,
        2.0,
        [0.98, 0.9, 0.55, 1.0],
    );

    // Power bars.
    let power_y = GAME_HEIGHT - 10.0;
    let power_w = 124.0;
    for (who, x) in [(0usize, 6.0), (1usize, GAME_WIDTH - 6.0 - power_w)] {
        let ratio = (sim.fighters[who].power / MAX_POWER).clamp(0.0, 1.0);
        painter.rect(
            Blend::Normal,
            x - 1.0,
            power_y - 1.0,
            power_w + 2.0,
            7.0,
            [0.05, 0.05, 0.07, 0.9],
        );
        painter.rect(
            Blend::Normal,
            x,
            power_y,
            power_w,
            5.0,
            [0.08, 0.12, 0.24, 1.0],
        );
        let fill = (power_w * ratio).round();
        let color = if ratio >= 1.0 {
            [0.95, 0.75, 0.2, 1.0]
        } else {
            [0.28, 0.55, 0.95, 1.0]
        };
        if who == 0 {
            painter.rect(Blend::Normal, x, power_y, fill, 5.0, color);
        } else {
            painter.rect(Blend::Normal, x + power_w - fill, power_y, fill, 5.0, color);
        }
    }

    // Round / result banner.
    let show_banner = sim.round.phase != RoundPhase::Fight && !sim.round.display.is_empty();
    if show_banner {
        let scale = 2.0;
        let text = &sim.round.display;
        let width = Painter::text_width(text, scale);
        let center_y = GAME_HEIGHT * 0.45;
        painter.rect(
            Blend::Normal,
            GAME_WIDTH * 0.5 - width * 0.5 - 4.0,
            center_y - 3.0,
            width + 8.0,
            GLYPH_H * scale + 6.0,
            [0.02, 0.02, 0.04, 0.72],
        );
        painter.text_centered(
            font,
            text,
            GAME_WIDTH * 0.5,
            center_y,
            scale,
            [1.0, 0.92, 0.55, 1.0],
        );
    }
}
