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
    pub screen_fonts: &'a std::collections::BTreeMap<u32, crate::screenfont::ScreenFont>,
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
    let hide_bg = sim.background_hidden();
    let hide_fg = sim.foreground_hidden();

    for (index, layer) in stage
        .render_layers()
        .enumerate()
        .filter(|(_, layer)| layer.layer_no < 0 && !hide_bg)
    {
        if let Some(state) = sim.stage_layers.get(index) {
            painter.set_scissor(layer_scissor(layer));
            painter.set_mask(layer.mask);
            draw_bg_layer(painter, deps.atlas, sim, stage, layer, state);
            painter.set_mask(false);
            painter.set_scissor(None);
        }
    }
    for (index, layer) in stage
        .render_layers()
        .enumerate()
        .filter(|(_, layer)| layer.layer_no == 0 && !hide_bg)
    {
        if let Some(state) = sim.stage_layers.get(index) {
            painter.set_scissor(layer_scissor(layer));
            painter.set_mask(layer.mask);
            draw_bg_layer(painter, deps.atlas, sim, stage, layer, state);
            painter.set_mask(false);
            painter.set_scissor(None);
        }
    }

    if !hide_bg {
        draw_reflections(painter, deps.atlas, sim);
    }
    draw_shadows(painter, sim);

    if let Some(env_color) = sim.env_color.filter(|color| color.under) {
        draw_env_color(painter, env_color);
    }

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

    for (index, layer) in stage
        .render_layers()
        .enumerate()
        .filter(|(_, layer)| layer.layer_no > 0 && !hide_fg)
    {
        if let Some(state) = sim.stage_layers.get(index) {
            painter.set_scissor(layer_scissor(layer));
            painter.set_mask(layer.mask);
            draw_bg_layer(painter, deps.atlas, sim, stage, layer, state);
            painter.set_mask(false);
            painter.set_scissor(None);
        }
    }

    let mut front: Vec<&Effect> = sim.effects.iter().filter(|e| !e.behind).collect();
    front.sort_by_key(|e| e.sprite_priority);
    for effect in front {
        draw_effect(painter, deps.atlas, sim, effect);
    }

    if let Some(screenpack) = &sim.assets.screenpack {
        let fight_deps = crate::fightscreen::FightDeps {
            atlas: deps.atlas,
            font: deps.font,
            screen_fonts: deps.screen_fonts,
        };
        crate::fightscreen::draw_background(painter, sim, &fight_deps, screenpack);
    }

    if let Some(env_color) = sim.env_color.filter(|color| !color.under) {
        draw_env_color(painter, env_color);
    }

    if let Some(screenpack) = &sim.assets.screenpack {
        let fight_deps = crate::fightscreen::FightDeps {
            atlas: deps.atlas,
            font: deps.font,
            screen_fonts: deps.screen_fonts,
        };
        crate::fightscreen::draw_foreground(painter, sim, &fight_deps, screenpack);
    } else {
        draw_hud(painter, deps.font, sim);
    }
}

fn draw_env_color(painter: &mut Painter, env_color: manato_sim::EnvColor) {
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

fn layer_scissor(layer: &manato_formats::stage::BgLayer) -> Option<[u32; 4]> {
    let [x1, y1, x2, y2] = layer.window;
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some([
        x1.max(0.0) as u32,
        y1.max(0.0) as u32,
        (x2 - x1).max(0.0) as u32,
        (y2 - y1).max(0.0) as u32,
    ])
}

fn draw_bg_layer(
    painter: &mut Painter,
    atlas: &SpriteAtlas,
    sim: &SimState,
    stage: &Stage,
    layer: &manato_formats::stage::BgLayer,
    state: &manato_sim::StageLayerState,
) {
    if !state.visible || !state.enabled {
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
        state.offset.x + sin_offset(state.sin_x) + cos_offset(state.cos_x),
        state.offset.y + sin_offset(state.sin_y) + cos_offset(state.cos_y),
    );
    let base = Vec2::new(
        layer.start.x - camera.x * layer.delta.x + camera.shake.x + motion.x,
        layer.start.y - camera.y * layer.delta.y + camera.shake.y + motion.y,
    );

    let action = state.action.or(layer.action);
    let (sprite, pose, anim) = if let Some(action) = action {
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
        let zoom = sim.camera.scale;
        draw_sprite(
            painter,
            atlas,
            bank::CHARACTER_BASE + projectile.owner as u8,
            sprite,
            sim.screen_pos(projectile.pos),
            pose.offset,
            projectile.facing,
            Vec2::new(pose.scale.x * zoom, pose.scale.y * zoom),
            pose.flip_h,
            pose.flip_v,
            pose.blend,
            [1.0, 1.0, 1.0, pose.alpha],
            pose.angle,
        );
    }
}

fn draw_reflections(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState) {
    let reflection = &sim.assets.stage.reflection;
    if reflection.intensity <= 0 {
        return;
    }
    let alpha = (reflection.intensity as f32 / 255.0).clamp(0.0, 1.0) * 0.45;
    for (who, fighter) in sim
        .fighters
        .iter()
        .enumerate()
        .filter(|(_, fighter)| fighter.owner.is_none())
    {
        if !fighter.active {
            continue;
        }
        let character = &sim.assets.characters[fighter.char_id];
        let Some(frame) = fighter.current_frame(sim.anim_character(who)) else {
            continue;
        };
        let Some(sprite) = character.sprites.get(frame.group, frame.index) else {
            continue;
        };
        let pose = Pose::from_frame(frame);
        let mut anchor = sim.screen_pos(fighter.pos);
        anchor.y = sim.ground_screen_y() * 2.0 - anchor.y;
        let base_scale = Vec2::new(
            character.constants.get_or("size.xscale", 1.0) * fighter.size_xscale,
            character.constants.get_or("size.yscale", 1.0) * fighter.size_yscale,
        );
        draw_sprite(
            painter,
            atlas,
            bank::CHARACTER_BASE + fighter.char_id as u8,
            sprite,
            anchor,
            pose.offset,
            fighter.facing,
            Vec2::new(
                base_scale.x * pose.scale.x * sim.camera.scale,
                base_scale.y * pose.scale.y * sim.camera.scale,
            ),
            pose.flip_h,
            !pose.flip_v,
            pose.blend,
            [
                reflection.color[0] as f32 / 255.0,
                reflection.color[1] as f32 / 255.0,
                reflection.color[2] as f32 / 255.0,
                alpha,
            ],
            pose.angle,
        );
    }
}

fn draw_shadows(painter: &mut Painter, sim: &SimState) {
    if sim.shadows_hidden() {
        return;
    }
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
        let w = (fighter.width_back + fighter.width_front) * 1.35 * sim.camera.scale;
        let h = 7.0 * sim.camera.scale;
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
    let (dest, flipped) = sprite_dest(anchor, offset, sprite, facing, scale, flip_h, flip_v);
    draw_atlas_quad(
        painter,
        atlas_sprite.page,
        atlas_sprite.uv,
        dest,
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
    let Some(frame) = fighter.current_frame(sim.anim_character(who)) else {
        return;
    };
    let Some(sprite) = character.sprites.get(frame.group, frame.index) else {
        return;
    };
    let mut anchor = sim.screen_pos(fighter.pos);
    anchor.x += character.constants.get_or("draw.offset.x", 0.0) + fighter.draw_offset.x;
    anchor.y += character.constants.get_or("draw.offset.y", 0.0) + fighter.draw_offset.y;
    let base_scale = Vec2::new(
        character.constants.get_or("size.xscale", 1.0) * fighter.size_xscale,
        character.constants.get_or("size.yscale", 1.0) * fighter.size_yscale,
    );
    let pose = Pose::from_frame(frame);
    let zoom = sim.camera.scale;
    let scale = Vec2::new(
        base_scale.x * pose.scale.x * zoom,
        base_scale.y * pose.scale.y * zoom,
    );

    let anim_character = sim.anim_character(who);
    for ghost in &fighter.after_images {
        let Some(anim) = anim_character.animation(ghost.anim_no) else {
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
                base_scale.x * ghost_pose.scale.x * zoom,
                base_scale.y * ghost_pose.scale.y * zoom,
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

/// Returns the screen-space clip rect declared by `window`, if any.
fn effect_scissor(effect: &Effect) -> Option<[u32; 4]> {
    let [x1, y1, x2, y2] = effect.window?;
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some([
        x1.max(0.0) as u32,
        y1.max(0.0) as u32,
        (x2 - x1).max(0.0) as u32,
        (y2 - y1).max(0.0) as u32,
    ])
}

fn draw_effect(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState, effect: &Effect) {
    painter.set_scissor(effect_scissor(effect));
    draw_effect_body(painter, atlas, sim, effect);
    painter.set_scissor(None);
}

fn draw_effect_body(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState, effect: &Effect) {
    let Some((anim, bank)) = effect_animation(sim, effect) else {
        return;
    };
    // `animelem` shifts the animation start; `animfreeze` holds that frame.
    let elem = effect.anim_elem.max(1) as usize;
    let start = anim
        .frames
        .iter()
        .take(elem - 1)
        .map(|frame| frame.effective_duration())
        .sum::<i32>();
    let time = if effect.anim_freeze {
        start
    } else {
        effect.time + start
    };
    let Some((frame_idx, _)) = anim.frame_at(time) else {
        return;
    };
    let Some(frame) = anim.frames.get(frame_idx) else {
        return;
    };
    let Some(sprite) = effect_sprite(sim, effect, frame.group, frame.index) else {
        return;
    };
    let anchor = if effect.screen_space {
        effect.pos
    } else {
        sim.screen_pos(effect.pos)
    };
    let blend = match effect.blend {
        EffectBlend::Normal => Blend::Normal,
        EffectBlend::Add => Blend::Add,
        EffectBlend::Sub => Blend::Sub,
    };
    let pose = Pose::from_frame(frame);
    let zoom = if effect.screen_space {
        1.0
    } else {
        sim.camera.scale
    };
    let scale = Vec2::new(
        effect.scale.x * pose.scale.x * zoom,
        effect.scale.y * pose.scale.y * zoom,
    );
    let offset = Vec2::new(frame.offset.x, frame.offset.y);
    let flip_h = frame.flip_h ^ effect.flip_h;
    let flip_v = frame.flip_v ^ effect.flip_v;
    let color = [1.0, 1.0, 1.0, (effect.alpha * pose.alpha).clamp(0.0, 1.0)];
    let angle = pose.angle + effect.angle;

    if effect.shadow && !effect.screen_space {
        draw_effect_shadow(painter, sim, effect, sprite, scale);
    }

    let rotated = effect.x_angle != 0.0 || effect.y_angle != 0.0 || effect.x_shear != 0.0;
    if !rotated && effect.projection == manato_sim::ProjectionMode::Orthographic {
        draw_sprite(
            painter,
            atlas,
            bank,
            sprite,
            anchor,
            offset,
            effect.facing,
            scale,
            flip_h,
            flip_v,
            blend,
            color,
            angle,
        );
        return;
    }

    let Some(atlas_sprite) = atlas.get(bank, sprite.group, sprite.index) else {
        return;
    };
    let (dest, flipped) = sprite_dest(anchor, offset, sprite, effect.facing, scale, flip_h, flip_v);
    let localcoord_h = sim.assets.stage.localcoord.1.max(1.0);
    let focal = effect.focal_length * (GAME_HEIGHT / localcoord_h);
    let corners = project_quad(
        dest,
        angle,
        effect.x_angle,
        effect.y_angle,
        effect.x_shear,
        effect.projection,
        focal,
        Vec2::new(GAME_WIDTH * 0.5, GAME_HEIGHT * 0.5),
    );
    painter.quad_corners(
        TexRef::Atlas(atlas_sprite.page),
        blend,
        corners,
        atlas_sprite.uv,
        flipped,
        flip_v,
        color,
    );
}

/// Ground shadow cast by an explod (`shadow = 1`). Matches the fighter shadow
/// treatment: an ellipse at the stage floor, faded by height.
fn draw_effect_shadow(
    painter: &mut Painter,
    sim: &SimState,
    effect: &Effect,
    sprite: &Sprite,
    scale: Vec2,
) {
    let shadow = &sim.assets.stage.shadow;
    if shadow.intensity <= 0 {
        return;
    }
    let ground = sim.screen_pos(Vec2::new(effect.pos.x, 0.0));
    let height = (-effect.pos.y).max(0.0);
    let base_alpha = (shadow.intensity as f32 / 255.0).clamp(0.0, 1.0) * 0.5;
    let alpha = (base_alpha - height / 400.0).clamp(0.04, base_alpha.max(0.05));
    let w = sprite.width as f32 * scale.x.abs() * 0.8;
    let h = 7.0 * sim.camera.scale * effect.scale.y.abs().max(0.05);
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

/// Computes the quad destination and horizontal-flip flag for a sprite.
#[allow(clippy::too_many_arguments)]
fn sprite_dest(
    anchor: Vec2,
    offset: Vec2,
    sprite: &Sprite,
    facing: i32,
    scale: Vec2,
    flip_h: bool,
    flip_v: bool,
) -> ([f32; 4], bool) {
    let w = sprite.width as f32 * scale.x;
    let h = sprite.height as f32 * scale.y;
    let axis_x = sprite.axis_x as f32 * scale.x;
    let axis_y = sprite.axis_y as f32 * scale.y;
    let offset_x = offset.x * scale.x;
    let offset_y = offset.y * scale.y;
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
    ([x0, y0, x0 + w, y0 + h], facing_flip ^ flip_h)
}

/// Applies shear, 3D rotation, and projection to a quad. `docs/spec/render.md`
/// §4.1. Corners are top-left, top-right, bottom-right, bottom-left.
#[allow(clippy::too_many_arguments)]
fn project_quad(
    dest: [f32; 4],
    angle: f32,
    x_angle: f32,
    y_angle: f32,
    x_shear: f32,
    projection: manato_sim::ProjectionMode,
    focal: f32,
    view_center: Vec2,
) -> [[f32; 2]; 4] {
    use manato_sim::ProjectionMode;
    let [x0, y0, x1, y1] = dest;
    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.5;
    let (sz, cz) = angle.to_radians().sin_cos();
    let (sx, cxx) = x_angle.to_radians().sin_cos();
    let (sy, cyy) = y_angle.to_radians().sin_cos();
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut out = [[0.0f32; 2]; 4];
    for (slot, (x, y)) in corners.into_iter().enumerate() {
        // Shear around the quad centre.
        let mut lx = x - cx - x_shear * (y - cy);
        let mut ly = y - cy;
        let mut lz = 0.0f32;
        // Rz, then Rx, then Ry.
        (lx, ly) = (lx * cz - ly * sz, lx * sz + ly * cz);
        (ly, lz) = (ly * cxx - lz * sx, ly * sx + lz * cxx);
        (lx, lz) = (lx * cyy + lz * sy, -lx * sy + lz * cyy);

        let (px, py) = match projection {
            ProjectionMode::Orthographic => (cx + lx, cy + ly),
            ProjectionMode::Perspective => {
                let k = focal / (focal - lz).max(1.0);
                (cx + lx * k, cy + ly * k)
            }
            ProjectionMode::Perspective2 => {
                let k = focal / (focal - lz).max(1.0);
                (
                    view_center.x + (cx + lx - view_center.x) * k,
                    view_center.y + (cy + ly - view_center.y) * k,
                )
            }
        };
        out[slot] = [px, py];
    }
    out
}

fn draw_hud(painter: &mut Painter, font: &FontAtlas, sim: &SimState) {
    let hide_bars = sim.bars_hidden();
    let bar_w = 148.0;
    let bar_h = 10.0;
    let y = 8.0;
    let p1_x = 6.0;
    let p2_x = GAME_WIDTH - 6.0 - bar_w;

    for (who, x) in [(0usize, p1_x), (1usize, p2_x)] {
        if hide_bars {
            continue;
        }
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
    if hide_bars {
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use manato_sim::ProjectionMode;

    #[test]
    fn orthographic_zero_rotation_is_identity() {
        let dest = [0.0, 0.0, 10.0, 10.0];
        let corners = project_quad(
            dest,
            0.0,
            0.0,
            0.0,
            0.0,
            ProjectionMode::Orthographic,
            2048.0,
            Vec2::new(160.0, 120.0),
        );
        assert_eq!(corners[0], [0.0, 0.0]);
        assert_eq!(corners[1], [10.0, 0.0]);
        assert_eq!(corners[2], [10.0, 10.0]);
        assert_eq!(corners[3], [0.0, 10.0]);
    }

    #[test]
    fn angle_rotates_about_quad_centre() {
        let dest = [0.0, 0.0, 10.0, 10.0];
        let corners = project_quad(
            dest,
            90.0,
            0.0,
            0.0,
            0.0,
            ProjectionMode::Orthographic,
            2048.0,
            Vec2::new(160.0, 120.0),
        );
        // Top-left rotates to the top-right position and so on.
        assert!((corners[0][0] - 10.0).abs() < 0.001);
        assert!((corners[0][1] - 0.0).abs() < 0.001);
        assert!((corners[2][0] - 0.0).abs() < 0.001);
        assert!((corners[2][1] - 10.0).abs() < 0.001);
    }

    #[test]
    fn x_angle_collapses_height_and_keeps_centre() {
        let dest = [0.0, 0.0, 10.0, 10.0];
        let corners = project_quad(
            dest,
            0.0,
            90.0,
            0.0,
            0.0,
            ProjectionMode::Orthographic,
            2048.0,
            Vec2::new(160.0, 120.0),
        );
        for corner in corners {
            assert!((corner[1] - 5.0).abs() < 0.001);
        }
    }

    #[test]
    fn perspective_scales_about_the_view_centre() {
        let dest = [10.0, 20.0, 30.0, 40.0];
        let view = Vec2::new(160.0, 120.0);
        let corners = project_quad(
            dest,
            0.0,
            60.0,
            0.0,
            0.0,
            ProjectionMode::Perspective2,
            2048.0,
            view,
        );
        // A non-zero x angle moves corners in Z, so the projected quad is
        // smaller than the source quad about the view centre.
        let width = corners[1][0] - corners[0][0];
        assert!(width < 20.0);
        let centre_x = (corners[0][0] + corners[2][0]) * 0.5;
        assert!((centre_x - 20.0).abs() < 2.0);
    }

    #[test]
    fn shear_skews_without_moving_the_centre() {
        let dest = [0.0, 0.0, 10.0, 10.0];
        let corners = project_quad(
            dest,
            0.0,
            0.0,
            0.0,
            0.5,
            ProjectionMode::Orthographic,
            2048.0,
            Vec2::new(160.0, 120.0),
        );
        // Bottom corners move left, top corners move right.
        assert!(corners[3][0] < corners[0][0]);
        let centre_x = (corners[0][0] + corners[2][0]) * 0.5;
        assert!((centre_x - 5.0).abs() < 0.001);
    }
}
