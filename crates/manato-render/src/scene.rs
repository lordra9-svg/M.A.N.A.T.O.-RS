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
    /// Atlas banks for in-match overlay scene backgrounds.
    pub overlay_bg_banks: &'a std::collections::HashMap<(usize, usize), u8>,
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

    /// Pose from a frame plus interpolated offset/scale/angle/alpha.
    fn from_pose(frame: &AnimFrame, pose: &manato_formats::FramePose) -> Self {
        Self {
            offset: pose.offset,
            scale: pose.scale,
            angle: pose.angle,
            alpha: (pose.src_alpha / 255.0).clamp(0.0, 1.0),
            ..Self::from_frame(frame)
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

/// Applies a global `AllPalFX`/`BgPalFX` to a sprite color.
fn global_tint(color: [f32; 4], pal: &manato_sim::GlobalPalFx) -> [f32; 4] {
    if !pal.active() {
        return color;
    }
    [
        (color[0] * pal.mul[0] + pal.add[0] / 255.0).clamp(0.0, 1.0),
        (color[1] * pal.mul[1] + pal.add[1] / 255.0).clamp(0.0, 1.0),
        (color[2] * pal.mul[2] + pal.add[2] / 255.0).clamp(0.0, 1.0),
        (color[3] * pal.alpha).clamp(0.0, 1.0),
    ]
}

pub fn build(painter: &mut Painter, sim: &SimState, deps: &SceneDeps) {
    painter.clear();
    let stage = &sim.assets.stage;
    let hide_bg = sim.background_hidden();
    let hide_fg = sim.foreground_hidden();

    let effective_layer_no = |index: usize, layer: &manato_formats::stage::BgLayer| {
        sim.stage_layers
            .get(index)
            .and_then(|state| state.layer_no)
            .unwrap_or(layer.layer_no)
    };
    // Scene-wide palette filter: characters/effects use `AllPalFX`, stage
    // layers switch to `BgPalFX` inside their loops.
    painter.set_global_palfx(sim.all_pal.color, sim.all_pal.hue, sim.all_pal.invert);
    painter.set_global_palfx(sim.bg_pal.color, sim.bg_pal.hue, sim.bg_pal.invert);
    for (index, layer) in stage
        .render_layers()
        .enumerate()
        .filter(|(index, layer)| effective_layer_no(*index, layer) < 0 && !hide_bg)
    {
        if let Some(state) = sim.stage_layers.get(index) {
            let restore = painter_depth(painter, state);
            painter.set_scissor(layer_scissor(layer));
            painter.set_mask(layer.mask);
            draw_bg_layer(painter, deps.atlas, sim, stage, layer, state);
            painter.set_mask(false);
            painter.set_scissor(None);
            painter.set_depth(restore);
        }
    }
    for (index, layer) in stage
        .render_layers()
        .enumerate()
        .filter(|(index, layer)| effective_layer_no(*index, layer) == 0 && !hide_bg)
    {
        if let Some(state) = sim.stage_layers.get(index) {
            let restore = painter_depth(painter, state);
            painter.set_scissor(layer_scissor(layer));
            painter.set_mask(layer.mask);
            draw_bg_layer(painter, deps.atlas, sim, stage, layer, state);
            painter.set_mask(false);
            painter.set_scissor(None);
            painter.set_depth(restore);
        }
    }
    painter.set_global_palfx(sim.all_pal.color, sim.all_pal.hue, sim.all_pal.invert);

    if !hide_bg {
        draw_reflections(painter, deps.atlas, sim);
    }
    draw_shadows(painter, sim);

    if let Some(env_color) = sim.env_color.filter(|color| color.under) {
        draw_env_color(painter, env_color);
    }

    // Effects that are explicitly behind characters.
    painter.set_depth(0.9);
    let mut behind: Vec<&Effect> = sim.effects.iter().filter(|e| e.behind).collect();
    behind.sort_by_key(|e| e.sprite_priority);
    for effect in behind {
        draw_effect(painter, deps.atlas, sim, effect);
    }

    painter.set_depth(0.89);
    draw_texts(painter, deps, sim, true);

    let mut order: Vec<usize> = (0..sim.fighters.len()).collect();
    order.sort_by_key(|&i| sim.fighters[i].sprite_priority);
    for who in order {
        painter.set_shader(sim.fighters[who].shader.clone());
        painter.set_shader_params(Some(sim.fighters[who].shader_params));
        let bank = bank::CHARACTER_BASE + sim.fighters[who].char_id as u8;
        painter.set_shader_textures(
            sim.fighters[who]
                .shader_tex1
                .map(|(group, index)| (bank, group, index)),
            sim.fighters[who]
                .shader_tex2
                .map(|(group, index)| (bank, group, index)),
        );
        draw_fighter(painter, deps.atlas, sim, who);
        painter.set_shader(None);
        painter.set_shader_params(None);
        painter.set_shader_textures(None, None);
    }

    painter.set_depth(0.45);
    draw_projectiles(painter, deps.atlas, sim, false);

    painter.set_depth(0.1);
    painter.set_global_palfx(sim.bg_pal.color, sim.bg_pal.hue, sim.bg_pal.invert);
    for (index, layer) in stage
        .render_layers()
        .enumerate()
        .filter(|(index, layer)| effective_layer_no(*index, layer) > 0 && !hide_fg)
    {
        if let Some(state) = sim.stage_layers.get(index) {
            let restore = painter_depth(painter, state);
            painter.set_scissor(layer_scissor(layer));
            painter.set_mask(layer.mask);
            draw_bg_layer(painter, deps.atlas, sim, stage, layer, state);
            painter.set_mask(false);
            painter.set_scissor(None);
            painter.set_depth(restore);
        }
    }
    painter.set_global_palfx(sim.all_pal.color, sim.all_pal.hue, sim.all_pal.invert);

    // Projectiles with `projlayerno > 0` draw over the stage foreground.
    painter.set_depth(0.08);
    draw_projectiles(painter, deps.atlas, sim, true);

    painter.set_depth(0.06);
    let mut front: Vec<&Effect> = sim.effects.iter().filter(|e| !e.behind).collect();
    front.sort_by_key(|e| e.sprite_priority);
    for effect in front {
        draw_effect(painter, deps.atlas, sim, effect);
    }

    painter.set_depth(0.05);
    draw_texts(painter, deps, sim, false);
    draw_storyboard_overlays(painter, deps, sim);

    // Fight-screen HUD is closest to the viewer and is not palette-filtered.
    painter.reset_global_palfx();
    painter.set_depth(0.02);
    if let Some(screenpack) = &sim.assets.screenpack {
        let fight_deps = crate::fightscreen::FightDeps {
            atlas: deps.atlas,
            font: deps.font,
            screen_fonts: deps.screen_fonts,
        };
        crate::fightscreen::draw_background(painter, sim, &fight_deps, screenpack);
    }

    painter.set_depth(0.01);
    if let Some(env_color) = sim.env_color.filter(|color| !color.under) {
        draw_env_color(painter, env_color);
    }

    painter.set_depth(0.0);
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

/// Draws `Text` objects. Negative `layerno` values go behind the fighters.
fn draw_texts(painter: &mut Painter, deps: &SceneDeps, sim: &SimState, behind: bool) {
    for text in &sim.texts {
        if (text.layer_no < 0) != behind {
            continue;
        }
        let visible = text.visible_text();
        if visible.is_empty() {
            continue;
        }
        let tint = [
            text.color[0] as f32 / 255.0,
            text.color[1] as f32 / 255.0,
            text.color[2] as f32 / 255.0,
            text.color[3] as f32 / 255.0,
        ];
        let stage = &sim.assets.stage;
        let z_scale = stage.z_scale(text.z);
        let mut pos = text.pos;
        if (z_scale - 1.0).abs() > f32::EPSILON {
            pos = project_depth(
                pos,
                z_scale,
                stage.z_y_offset(text.z - sim.camera.z),
                GAME_WIDTH * 0.5,
                sim.ground_screen_y(),
            );
        }
        let scale = text.scale * z_scale;
        let restore = painter.depth();
        if text.z != 0.0 {
            painter.set_depth((0.5 + text.z * 0.001).clamp(0.05, 0.95));
        }
        if text.font >= 0 {
            if let Some(font) = deps.screen_fonts.get(&(text.font as u32)) {
                font.draw(painter, visible, pos, scale, text.align, tint);
                painter.set_depth(restore);
                continue;
            }
        }
        let pixel_scale = scale.y.max(scale.x).clamp(0.4, 4.0);
        let width = Painter::text_width(visible, pixel_scale);
        let x = match text.align {
            1 => pos.x - width * 0.5,
            2 => pos.x - width,
            _ => pos.x,
        };
        painter.text(deps.font, visible, x, pos.y, pixel_scale, tint);
        painter.set_depth(restore);
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

/// Effective `ModifyBGCtrl` palette filter for a stage layer at `time`.
/// Returns `(saturation, hue, invert)`.
fn bg_layer_palfx(state: &manato_sim::StageLayerState, time: f32) -> (f32, f32, bool) {
    let phase = |period: i32| {
        if period > 0 {
            (std::f32::consts::TAU * time / period as f32).sin()
        } else {
            0.0
        }
    };
    let color =
        (state.pal_color + phase(state.pal_sin_color_period) * state.pal_sin_color).clamp(0.0, 1.0);
    let hue =
        (state.pal_hue + phase(state.pal_sin_hue_period) * state.pal_sin_hue).clamp(-0.5, 0.5);
    (color, hue, state.pal_invert)
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
    let pos = state.pos.unwrap_or(Vec2::ZERO);
    let motion = Vec2::new(
        state.offset.x + pos.x + sin_offset(state.sin_x) + cos_offset(state.cos_x),
        state.offset.y + pos.y + sin_offset(state.sin_y) + cos_offset(state.cos_y),
    );
    let start = state.start.unwrap_or(layer.start);
    let delta = state.delta.unwrap_or(layer.delta);
    let mut scale = state.scale.unwrap_or(Vec2::new(1.0, 1.0));
    let mut base = Vec2::new(
        start.x - camera.x * delta.x + camera.shake.x + motion.x,
        start.y - camera.y * delta.y + camera.shake.y + motion.y,
    );
    // `ModifyBGCtrl3D` depth perspective for this layer.
    if let Some(z) = state.z {
        let z_scale = stage.z_scale(z);
        base = project_depth(
            base,
            z_scale,
            stage.z_y_offset(z - sim.camera.z),
            GAME_WIDTH * 0.5,
            sim.ground_screen_y(),
        );
        scale = scale * z_scale;
    }
    if let Some(extra) = state.z_scale {
        scale = Vec2::new(scale.x * extra.x, scale.y * extra.y);
    }

    let action = state.action.or(layer.action);
    let (sprite, pose, anim) = if let Some(action) = action {
        if let Some(anim) = stage.animations.get(&action) {
            let Some(interpolated) = anim.pose_at(sim.bg_time as i32) else {
                return;
            };
            let Some(frame) = anim.frames.get(interpolated.frame) else {
                return;
            };
            let (group, index) = (frame.group, frame.index);
            let pose = Pose::from_pose(frame, &interpolated);
            let Some(sprite) = stage.sprites.get(group, index) else {
                return;
            };
            (sprite, pose, Some(anim))
        } else {
            return;
        }
    } else if let Some((group, index)) = state.sprite.or(layer.sprite) {
        let Some(sprite) = stage.sprites.get(group, index) else {
            return;
        };
        (
            sprite,
            Pose {
                offset: Vec2::ZERO,
                flip_h: false,
                flip_v: false,
                blend: blend_of(state.trans.unwrap_or(layer.trans)),
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

    let blend = blend_of(state.trans.unwrap_or(layer.trans));
    let xscale = layer.xscale * scale.x;
    let yscale = layer.yscale * scale.y;
    let (w, h) = (sprite.width as f32 * xscale, sprite.height as f32 * yscale);
    let base_left = base.x
        + if pose.flip_h {
            pose.offset.x - (sprite.width as f32 * xscale - sprite.axis_x as f32 * xscale)
        } else {
            pose.offset.x - sprite.axis_x as f32 * xscale
        };
    let base_top = base.y
        + if pose.flip_v {
            pose.offset.y - (sprite.height as f32 * yscale - sprite.axis_y as f32 * yscale)
        } else {
            pose.offset.y - sprite.axis_y as f32 * yscale
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
    let (saturation, hue, invert) = bg_layer_palfx(state, sim.bg_time);
    painter.set_palfx([0.0, 0.0, 0.0], saturation, hue, invert);
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
                global_tint([1.0, 1.0, 1.0, pose.alpha], &sim.bg_pal),
                xscale * pose.scale.x,
                yscale * pose.scale.y,
                pose.angle,
            );
        }
    }
    painter.reset_palfx();
}

/// Indices of projectiles drawn in the background (`foreground = false`) or
/// foreground layer, stable-sorted by `projsprpriority` then insertion order.
pub fn projectile_draw_order(
    projectiles: &[manato_sim::projectile::Projectile],
    foreground: bool,
) -> Vec<usize> {
    let mut order: Vec<usize> = projectiles
        .iter()
        .enumerate()
        .filter(|(_, projectile)| (projectile.layer_no > 0) == foreground)
        .map(|(index, _)| index)
        .collect();
    order.sort_by_key(|&index| (projectiles[index].sprite_priority, index));
    order
}

fn draw_projectiles(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState, foreground: bool) {
    for index in projectile_draw_order(&sim.projectiles, foreground) {
        let projectile = &sim.projectiles[index];
        let char = &sim.assets.characters[sim.fighters[projectile.owner].char_id];
        let Some(anim) = char.animation(projectile.anim) else {
            continue;
        };
        let Some(interpolated) = anim.pose_at(projectile.time) else {
            continue;
        };
        let Some(frame) = anim.frames.get(interpolated.frame) else {
            continue;
        };
        let Some(sprite) = char.sprites.get(frame.group, frame.index) else {
            continue;
        };
        let pose = Pose::from_pose(frame, &interpolated);
        let zoom = sim.camera.scale;
        let stage = &sim.assets.stage;
        let (anchor, scale) = depth_transform(
            sim.screen_pos(projectile.pos),
            Vec2::new(
                pose.scale.x * projectile.scale.x * zoom,
                pose.scale.y * projectile.scale.y * zoom,
            ),
            projectile.z,
            sim.camera.z,
            stage,
            GAME_WIDTH * 0.5,
            sim.ground_screen_y(),
        );
        let restore = painter.depth();
        if projectile.z != 0.0 {
            painter.set_depth((0.5 + projectile.z * 0.001).clamp(0.05, 0.95));
        }
        let angle = pose.angle + projectile.angle * projectile.facing as f32;
        let bank = bank::CHARACTER_BASE + projectile.owner as u8;
        let color = global_tint([1.0, 1.0, 1.0, pose.alpha], &sim.all_pal);

        if projectile.x_shear != 0.0
            || projectile.projection != manato_sim::ProjectionMode::Orthographic
        {
            let Some(atlas_sprite) = atlas.get(bank, sprite.group, sprite.index) else {
                continue;
            };
            let (dest, flipped) = sprite_dest(
                anchor,
                pose.offset,
                sprite,
                projectile.facing,
                scale,
                pose.flip_h,
                pose.flip_v,
            );
            let localcoord_h = sim.assets.stage.localcoord.1.max(1.0);
            let focal = projectile.focal_length * (GAME_HEIGHT / localcoord_h);
            let corners = project_quad(
                dest,
                angle,
                0.0,
                0.0,
                projectile.x_shear,
                projectile.projection,
                focal,
                Vec2::new(GAME_WIDTH * 0.5, GAME_HEIGHT * 0.5),
            );
            painter.quad_corners(
                TexRef::Atlas(atlas_sprite.page),
                pose.blend,
                corners,
                atlas_sprite.uv,
                flipped,
                pose.flip_v,
                color,
            );
            painter.set_depth(restore);
            continue;
        }

        draw_sprite(
            painter,
            atlas,
            bank,
            sprite,
            anchor,
            pose.offset,
            projectile.facing,
            scale,
            pose.flip_h,
            pose.flip_v,
            pose.blend,
            color,
            angle,
        );
    }
}

fn draw_reflections(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState) {
    let reflection = &sim.assets.stage.reflection;
    for (who, fighter) in sim
        .fighters
        .iter()
        .enumerate()
        .filter(|(_, fighter)| fighter.owner.is_none())
    {
        if !fighter.active {
            continue;
        }
        let intensity = fighter.reflection_intensity.unwrap_or(reflection.intensity);
        if intensity <= 0 {
            continue;
        }
        let color = fighter.reflection_color.unwrap_or(reflection.color);
        let alpha = (intensity as f32 / 255.0).clamp(0.0, 1.0) * 0.45;
        let xscale = fighter
            .reflection_xscale
            .filter(|value| *value > 0.0)
            .unwrap_or(1.0);
        let yscale = fighter
            .reflection_yscale
            .filter(|value| *value > 0.0)
            .unwrap_or(1.0);
        let character = &sim.assets.characters[fighter.char_id];
        let Some(frame) = fighter.current_frame(sim.anim_character(who)) else {
            continue;
        };
        let Some(sprite) = fighter.sprite_for(character, frame.group, frame.index) else {
            continue;
        };
        let pose = Pose::from_frame(frame);
        let stage = &sim.assets.stage;
        let ground = sim.ground_screen_y();
        let (mut anchor, z_scale) = depth_transform(
            sim.screen_pos(fighter.pos) + fighter.reflection_offset * sim.camera.scale,
            Vec2::new(1.0, 1.0),
            fighter.z,
            sim.camera.z,
            stage,
            GAME_WIDTH * 0.5,
            ground,
        );
        let z_scale = z_scale.x;
        anchor.y = ground * 2.0 - anchor.y;
        let base_scale = Vec2::new(
            character.constants.get_or("size.xscale", 1.0) * fighter.size_xscale,
            character.constants.get_or("size.yscale", 1.0) * fighter.size_yscale,
        ) * z_scale;
        let restore = painter.depth();
        if fighter.z != 0.0 {
            painter.set_depth((0.5 + fighter.z * 0.001).clamp(0.05, 0.95));
        }
        draw_sprite(
            painter,
            atlas,
            bank::CHARACTER_BASE + fighter.char_id as u8,
            sprite,
            anchor,
            pose.offset,
            fighter.facing,
            Vec2::new(
                base_scale.x * pose.scale.x * sim.camera.scale * xscale,
                base_scale.y * pose.scale.y * sim.camera.scale * yscale,
            ),
            pose.flip_h,
            !pose.flip_v,
            pose.blend,
            [
                color[0] as f32 / 255.0,
                color[1] as f32 / 255.0,
                color[2] as f32 / 255.0,
                alpha,
            ],
            pose.angle,
        );
        painter.set_depth(restore);
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
        let shadow = &sim.assets.stage.shadow;
        let stage = &sim.assets.stage;
        let z_scale = stage.z_scale(fighter.z);
        let mut ground = sim.screen_pos(Vec2::new(fighter.pos.x, 0.0))
            + fighter.shadow_offset * sim.camera.scale;
        if (z_scale - 1.0).abs() > f32::EPSILON {
            ground = project_depth(
                ground,
                z_scale,
                stage.z_y_offset(fighter.z - sim.camera.z),
                GAME_WIDTH * 0.5,
                sim.ground_screen_y(),
            );
        }
        let height = (-fighter.pos.y).max(0.0);
        let intensity = fighter.shadow_intensity.unwrap_or(shadow.intensity);
        let color = fighter.shadow_color.unwrap_or(shadow.color);
        let xscale = fighter
            .shadow_xscale
            .filter(|value| *value > 0.0)
            .unwrap_or(1.0);
        let yscale = fighter
            .shadow_yscale
            .filter(|value| *value > 0.0)
            .unwrap_or(1.0);
        let base_alpha = (intensity as f32 / 255.0).clamp(0.0, 1.0) * 0.6;
        let alpha = (base_alpha - height / 400.0).clamp(0.05, base_alpha.max(0.05));
        let w =
            (fighter.width_back + fighter.width_front) * 1.35 * sim.camera.scale * xscale * z_scale;
        let h = 7.0 * sim.camera.scale * yscale * z_scale;
        let restore = painter.depth();
        if fighter.z != 0.0 {
            painter.set_depth((0.5 + fighter.z * 0.001).clamp(0.05, 0.95));
        }
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
                color[0] as f32 / 255.0,
                color[1] as f32 / 255.0,
                color[2] as f32 / 255.0,
                alpha,
            ],
        );
        painter.set_depth(restore);
    }
}

pub(crate) fn draw_sprite(
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
    painter.set_palette(sprite.palette_key.map(|key| (bank, key)));
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
pub(crate) fn draw_sprite_at(
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
    painter.set_palette(sprite.palette_key.map(|key| (bank, key)));
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

/// Applies stage `[Scaling]` perspective: scale about the camera centre and
/// the ground line, then shift by the depth-to-screen offset.
fn project_depth(anchor: Vec2, z_scale: f32, y_offset: f32, pivot_x: f32, ground: f32) -> Vec2 {
    Vec2::new(
        pivot_x + (anchor.x - pivot_x) * z_scale,
        ground + (anchor.y - ground) * z_scale + y_offset,
    )
}

/// Applies a layer's `ModifyBGCtrl3D` depth and returns the depth to restore.
fn painter_depth(painter: &mut Painter, state: &manato_sim::StageLayerState) -> f32 {
    let restore = painter.depth();
    if let Some(z) = state.z {
        painter.set_depth((0.5 + z * 0.001).clamp(0.05, 0.95));
    }
    restore
}

/// Applies stage depth perspective to an object anchor and scale.
fn depth_transform(
    anchor: Vec2,
    scale: Vec2,
    z: f32,
    camera_z: f32,
    stage: &Stage,
    pivot_x: f32,
    ground: f32,
) -> (Vec2, Vec2) {
    let z_scale = stage.z_scale(z);
    if (z_scale - 1.0).abs() <= f32::EPSILON {
        return (anchor, scale);
    }
    (
        project_depth(
            anchor,
            z_scale,
            stage.z_y_offset(z - camera_z),
            pivot_x,
            ground,
        ),
        scale * z_scale,
    )
}

/// Per-ghost palette parameters from `AfterImage`'s `pal*` options. `depth`
/// is 0 for the newest ghost and increases for older ones.
fn afterimage_palette(
    after: Option<&manato_sim::fighter::AfterImageState>,
    alpha: f32,
    depth: f32,
) -> ([f32; 4], f32, f32, bool) {
    let mut tint = [1.0, 1.0, 1.0, alpha];
    let mut saturation = 1.0;
    let mut hue = 0.0;
    let mut invert = false;
    if let Some(after) = after {
        for axis in 0..3 {
            let post = if depth > 0.0 {
                after.pal_post[axis]
            } else {
                0.0
            };
            let add = after.pal_add[axis] + after.pal_step_add[axis] * depth + post;
            let mul = after.pal_mul[axis] * after.pal_step_mul[axis].powf(depth);
            tint[axis] = (mul + add / 255.0).clamp(0.0, 1.0);
        }
        saturation = after.pal_color;
        hue = after.pal_hue;
        invert = after.pal_invert;
    }
    (tint, saturation, hue, invert)
}

fn draw_fighter(painter: &mut Painter, atlas: &SpriteAtlas, sim: &SimState, who: usize) {
    let fighter = &sim.fighters[who];
    // Positive `z` is farther; `0` matches the default mid-ground plane.
    painter.set_depth((0.5 + fighter.z * 0.001).clamp(0.05, 0.95));
    if !fighter.active || fighter.assert_flags & manato_sim::fighter::ASSERT_INVISIBLE != 0 {
        return;
    }
    let character: &Character = &sim.assets.characters[fighter.char_id];
    let anim_character = sim.anim_character(who);
    let Some(anim) = anim_character.animation(fighter.anim_no) else {
        return;
    };
    let Some(frame) = anim.frames.get(fighter.anim_frame) else {
        return;
    };
    let Some(sprite) = fighter.sprite_for(character, frame.group, frame.index) else {
        return;
    };
    let mut anchor = sim.screen_pos(fighter.pos);
    anchor.x += character.constants.get_or("draw.offset.x", 0.0) + fighter.draw_offset.x;
    anchor.y += character.constants.get_or("draw.offset.y", 0.0) + fighter.draw_offset.y;
    // Stage `[Scaling]` perspective: scale about the camera centre and the
    // ground line, then add the depth-to-screen Y offset.
    let stage = &sim.assets.stage;
    let z_scale = stage.z_scale(fighter.z);
    if (z_scale - 1.0).abs() > f32::EPSILON || fighter.z != 0.0 {
        anchor = project_depth(
            anchor,
            z_scale,
            stage.z_y_offset(fighter.z - sim.camera.z),
            manato_core::GAME_WIDTH * 0.5,
            sim.ground_screen_y(),
        );
    }
    let base_scale = Vec2::new(
        character.constants.get_or("size.xscale", 1.0) * fighter.size_xscale,
        character.constants.get_or("size.yscale", 1.0) * fighter.size_yscale,
    ) * z_scale;
    let elapsed = fighter.anim_elem_time(anim_character, fighter.anim_elem());
    let pose = anim
        .interpolated_pose(fighter.anim_frame, elapsed)
        .map(|interpolated| Pose::from_pose(frame, &interpolated))
        .unwrap_or_else(|| Pose::from_frame(frame));
    let zoom = sim.camera.scale;
    let scale = Vec2::new(
        base_scale.x * pose.scale.x * zoom,
        base_scale.y * pose.scale.y * zoom,
    );

    for (index, ghost) in fighter.after_images.iter().enumerate() {
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
        let depth = (fighter.after_images.len() - 1 - index) as f32;
        let (tint, saturation, hue, invert) =
            afterimage_palette(fighter.after_image.as_ref(), ghost.alpha, depth);
        painter.set_palfx([0.0, 0.0, 0.0], saturation, hue, invert);
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
            global_tint(tint, &sim.all_pal),
            ghost_pose.angle,
        );
        painter.reset_palfx();
    }

    let tint = global_tint(
        [
            (fighter.pal_mul[0] + fighter.pal_add[0] / 255.0).clamp(0.0, 1.0),
            (fighter.pal_mul[1] + fighter.pal_add[1] / 255.0).clamp(0.0, 1.0),
            (fighter.pal_mul[2] + fighter.pal_add[2] / 255.0).clamp(0.0, 1.0),
            (pose.alpha * fighter.pal_alpha * fighter.trans_alpha).clamp(0.0, 1.0),
        ],
        &sim.all_pal,
    );
    let (scale, angle) = if fighter.angle_draw {
        let angle = pose.angle + fighter.angle_draw_value.unwrap_or(fighter.draw_angle);
        let scale = fighter
            .angle_draw_scale
            .map(|extra| Vec2::new(scale.x * extra.x, scale.y * extra.y))
            .unwrap_or(scale);
        (scale, angle)
    } else {
        (scale, pose.angle)
    };
    let bank = bank::CHARACTER_BASE + fighter.char_id as u8;
    // Fighter `PalFX` color/hue/invert pass through the sprite shader; `add`
    // and `mul` are already folded into `tint`.
    painter.set_palfx(
        [0.0, 0.0, 0.0],
        fighter.pal_color,
        fighter.pal_hue,
        fighter.pal_invert,
    );
    if fighter.transform_x_shear != 0.0
        || fighter.transform_projection != manato_sim::ProjectionMode::Orthographic
    {
        if let Some(atlas_sprite) = atlas.get(bank, sprite.group, sprite.index) {
            let (dest, flipped) = sprite_dest(
                anchor,
                pose.offset,
                sprite,
                fighter.facing,
                scale,
                pose.flip_h,
                pose.flip_v,
            );
            let localcoord_h = sim.assets.stage.localcoord.1.max(1.0);
            let focal = fighter.transform_focal_length * (GAME_HEIGHT / localcoord_h);
            let corners = project_quad(
                dest,
                angle,
                0.0,
                0.0,
                fighter.transform_x_shear,
                fighter.transform_projection,
                focal,
                Vec2::new(GAME_WIDTH * 0.5, GAME_HEIGHT * 0.5),
            );
            painter.quad_corners(
                TexRef::Atlas(atlas_sprite.page),
                pose.blend,
                corners,
                atlas_sprite.uv,
                flipped,
                pose.flip_v,
                tint,
            );
        }
        painter.reset_palfx();
        return;
    }
    draw_sprite(
        painter,
        atlas,
        bank,
        sprite,
        anchor,
        pose.offset,
        fighter.facing,
        scale,
        pose.flip_h,
        pose.flip_v,
        pose.blend,
        tint,
        angle,
    );
    painter.reset_palfx();
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
    let restore = painter.depth();
    if effect.z != 0.0 {
        painter.set_depth((0.5 + effect.z * 0.001).clamp(0.05, 0.95));
    }
    painter.set_scissor(effect_scissor(effect));
    draw_effect_body(painter, atlas, sim, effect);
    painter.set_scissor(None);
    painter.set_depth(restore);
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
    let Some(interpolated) = anim.pose_at(time) else {
        return;
    };
    let Some(frame) = anim.frames.get(interpolated.frame) else {
        return;
    };
    let Some(sprite) = effect_sprite(sim, effect, frame.group, frame.index) else {
        return;
    };
    let stage = &sim.assets.stage;
    let z_scale = if effect.screen_space {
        1.0
    } else {
        stage.z_scale(effect.z)
    };
    let mut anchor = if effect.screen_space {
        effect.pos
    } else {
        sim.screen_pos(effect.pos)
    };
    if !effect.screen_space && (z_scale - 1.0).abs() > f32::EPSILON {
        anchor = project_depth(
            anchor,
            z_scale,
            stage.z_y_offset(effect.z - sim.camera.z),
            GAME_WIDTH * 0.5,
            sim.ground_screen_y(),
        );
    }
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
        effect.scale.x * pose.scale.x * zoom * z_scale,
        effect.scale.y * pose.scale.y * zoom * z_scale,
    );
    let offset = Vec2::new(frame.offset.x, frame.offset.y);
    let flip_h = frame.flip_h ^ effect.flip_h;
    let flip_v = frame.flip_v ^ effect.flip_v;
    let color = global_tint(
        [1.0, 1.0, 1.0, (effect.alpha * pose.alpha).clamp(0.0, 1.0)],
        &sim.all_pal,
    );
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
        let name = sim.assets.characters[sim.fighters[who].char_id].display_name();
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

        // Partner lifebars for standby team members.
        let roster = &sim.teams[who.min(1)];
        if roster.members.len() > 1 {
            let small_w = 60.0;
            let small_h = 4.0;
            for (index, member) in roster.members.iter().enumerate() {
                if index == roster.active {
                    continue;
                }
                let row = index.saturating_sub(if index > roster.active { 1 } else { 0 });
                let sy = y + bar_h + 16.0 + row as f32 * 7.0;
                let ratio = (member.life as f32 / member.life_max.max(1) as f32).clamp(0.0, 1.0);
                let sx = if who == 0 { x } else { x + bar_w - small_w };
                painter.rect(
                    Blend::Normal,
                    sx,
                    sy,
                    small_w,
                    small_h,
                    [0.18, 0.06, 0.06, 0.9],
                );
                let fill = (small_w * ratio).round();
                let fx = if who == 0 { sx } else { sx + small_w - fill };
                painter.rect(Blend::Normal, fx, sy, fill, small_h, [0.85, 0.55, 0.2, 0.9]);
            }
        }
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

/// Draws in-match `Storyboard` overlays (text layers).
/// Applies `palfx.invertblend` to a sprite's blend mode. Values `>= 1` swap
/// additive and subtractive passes, matching Ikemen's subtract handling.
fn palfx_blend(palfx: &manato_formats::storyboard::PalFxSample, blend: Blend) -> Blend {
    if !palfx.swaps_blend_passes() {
        return blend;
    }
    match blend {
        Blend::Add => Blend::Sub,
        Blend::Sub => Blend::Add,
        Blend::Normal => Blend::Normal,
    }
}

/// Negates PalFX add on swapped subtractive passes so the inversion is not
/// applied twice. Normal-blended quads are unchanged.
fn palfx_sprite_add(palfx: &manato_formats::storyboard::PalFxSample, blend: Blend) -> [f32; 3] {
    if palfx.swaps_blend_passes() && blend != Blend::Normal {
        [-palfx.add[0], -palfx.add[1], -palfx.add[2]]
    } else {
        palfx.add
    }
}

/// `invertblend = 3` relies on subtractive blending for inversion.
fn palfx_sprite_invert(palfx: &manato_formats::storyboard::PalFxSample, blend: Blend) -> bool {
    palfx.invert && !(palfx.disables_shader_invert() && blend != Blend::Normal)
}

fn draw_storyboard_overlays(painter: &mut Painter, deps: &SceneDeps, sim: &SimState) {
    for (index, overlay) in sim.storyboards.iter().enumerate() {
        let Some(bank_index) = bank::OVERLAY_BASE.checked_add(index as u8) else {
            continue;
        };
        let Some(scene) = overlay.player.current(&overlay.storyboard) else {
            continue;
        };
        let pos = overlay.pos;
        let overlay_scale = overlay.scale;
        let overlay_facing = overlay.facing;
        let overlay_alpha = overlay.script_alpha.clamp(0.0, 1.0);
        if let Some(color) = scene.clear_color {
            if scene.clear_alpha > 0 {
                painter.rect(
                    Blend::Normal,
                    0.0,
                    0.0,
                    GAME_WIDTH,
                    GAME_HEIGHT,
                    [
                        color[0] as f32 / 255.0,
                        color[1] as f32 / 255.0,
                        color[2] as f32 / 255.0,
                        scene.clear_alpha as f32 / 255.0,
                    ],
                );
            }
        }
        let overlay_key = (
            std::sync::Arc::as_ptr(&overlay.storyboard) as usize,
            overlay.player.scene,
        );
        if let (Some(stage), Some(bank_id)) =
            (&scene.bg, deps.overlay_bg_banks.get(&overlay_key).copied())
        {
            draw_storyboard_bg(
                painter,
                deps.atlas,
                stage,
                bank_id,
                overlay.player.time,
                Some((pos, overlay_scale, overlay_facing, overlay_alpha)),
            );
        }
        for layer in &scene.layers {
            if !layer.visible_at(overlay.player.time) {
                continue;
            }
            let local = layer.position_at(overlay.player.time);
            let local = local + scene.layer_all_pos;
            let base = Vec2::new(
                pos.x + local.x * overlay_scale.x * overlay_facing as f32,
                pos.y + local.y * overlay_scale.y,
            );
            let anchor = if overlay.screen_space {
                base
            } else {
                sim.screen_pos(base)
            };

            let sprite_pose = if layer.anim >= 0 {
                overlay
                    .storyboard
                    .animations
                    .get(&layer.anim)
                    .and_then(|anim| {
                        let local_time = (overlay.player.time - layer.start_time).max(0);
                        let interpolated = anim.pose_at(local_time)?;
                        let frame = anim.frames.get(interpolated.frame)?;
                        let sprite = overlay.storyboard.sprites.get(frame.group, frame.index)?;
                        Some((sprite, Pose::from_pose(frame, &interpolated)))
                    })
            } else if let Some((group, sprite_index)) = layer.sprite {
                overlay
                    .storyboard
                    .sprites
                    .get(group, sprite_index)
                    .map(|sprite| {
                        (
                            sprite,
                            Pose {
                                offset: Vec2::ZERO,
                                flip_h: false,
                                flip_v: false,
                                blend: Blend::Normal,
                                scale: Vec2::new(1.0, 1.0),
                                alpha: 1.0,
                                angle: 0.0,
                            },
                        )
                    })
            } else {
                None
            };
            if let Some((sprite, pose)) = sprite_pose {
                let scale = Vec2::new(
                    layer.scale.x * overlay_scale.x * pose.scale.x,
                    layer.scale.y * overlay_scale.y * pose.scale.y,
                );
                let palfx = layer.palfx_at(overlay.player.time);
                painter.set_palfx(
                    palfx_sprite_add(&palfx, pose.blend),
                    palfx.color,
                    palfx.hue,
                    palfx_sprite_invert(&palfx, pose.blend),
                );
                draw_sprite(
                    painter,
                    deps.atlas,
                    bank_index,
                    sprite,
                    anchor,
                    pose.offset,
                    layer.facing * overlay_facing,
                    scale,
                    pose.flip_h,
                    pose.flip_v,
                    palfx_blend(&palfx, pose.blend),
                    [
                        palfx.mul[0] / 256.0,
                        palfx.mul[1] / 256.0,
                        palfx.mul[2] / 256.0,
                        pose.alpha * overlay_alpha,
                    ],
                    pose.angle,
                );
                painter.reset_palfx();
            }

            if layer.text.is_some() {
                let text = layer.visible_text(overlay.player.time);
                if text.is_empty() {
                    continue;
                }
                let scale = (layer.scale.y * overlay.scale.y).clamp(0.4, 4.0);
                let palfx = layer.palfx_at(overlay.player.time);
                let tint = [
                    layer.color[0] as f32 / 255.0 * palfx.mul[0] / 256.0,
                    layer.color[1] as f32 / 255.0 * palfx.mul[1] / 256.0,
                    layer.color[2] as f32 / 255.0 * palfx.mul[2] / 256.0,
                    layer.color[3] as f32 / 255.0 * overlay_alpha,
                ];
                painter.set_palfx(palfx.add, palfx.color, palfx.hue, palfx.invert);
                painter.text(deps.font, &text, anchor.x, anchor.y, scale, tint);
                painter.reset_palfx();
            }
        }
        if let Some((color, alpha)) = overlay.player.fade_alpha(scene) {
            if alpha > 0 {
                painter.rect(
                    Blend::Normal,
                    0.0,
                    0.0,
                    GAME_WIDTH,
                    GAME_HEIGHT,
                    [
                        color[0] as f32 / 255.0,
                        color[1] as f32 / 255.0,
                        color[2] as f32 / 255.0,
                        alpha as f32 / 255.0,
                    ],
                );
            }
        }
    }
}

/// Draws one storyboard scene frame: clear colour, layers, text, and fades.
/// Storyboard backgrounds have no camera, so controllers are ticked from scene
/// start into a local layer-state snapshot.
pub(crate) fn storyboard_bg_states(
    stage: &manato_formats::stage::Stage,
    time: i32,
) -> Vec<manato_sim::StageLayerState> {
    let mut states: Vec<manato_sim::StageLayerState> = stage
        .layers
        .iter()
        .map(manato_sim::StageLayerState::from_layer)
        .collect();
    for tick in 0..=time.max(0) {
        manato_sim::update_bg_states(&stage.bg_ctrls, &mut states, tick, 1, 0);
    }
    states
}

fn draw_storyboard_bg(
    painter: &mut Painter,
    atlas: &SpriteAtlas,
    stage: &manato_formats::stage::Stage,
    bank_id: u8,
    time: i32,
    transform: Option<(Vec2, Vec2, i32, f32)>,
) {
    let states = storyboard_bg_states(stage, time);
    let sin_offset = |motion: [f32; 3]| {
        if motion[1].abs() > f32::EPSILON {
            motion[0]
                * (std::f32::consts::TAU * time.max(0) as f32 / motion[1] + motion[2].to_radians())
                    .sin()
        } else {
            0.0
        }
    };
    let cos_offset = |motion: [f32; 3]| {
        if motion[1].abs() > f32::EPSILON {
            motion[0]
                * (std::f32::consts::TAU * time.max(0) as f32 / motion[1] + motion[2].to_radians())
                    .cos()
        } else {
            0.0
        }
    };
    let mut layers: Vec<(
        &manato_formats::stage::BgLayer,
        &manato_sim::StageLayerState,
    )> = stage.render_layers().zip(states.iter()).collect();
    layers.sort_by_key(|(layer, state)| state.layer_no.unwrap_or(layer.layer_no));
    for (layer, state) in layers {
        if !state.visible || !state.enabled {
            continue;
        }
        let action = state.action.or(layer.action);
        let (sprite, pose) = if let Some(action) = action {
            let Some(anim) = stage.animations.get(&action) else {
                continue;
            };
            let Some(interpolated) = anim.pose_at(time.max(0)) else {
                continue;
            };
            let Some(frame) = anim.frames.get(interpolated.frame) else {
                continue;
            };
            let Some(sprite) = stage.sprites.get(frame.group, frame.index) else {
                continue;
            };
            (sprite, Pose::from_pose(frame, &interpolated))
        } else if let Some((group, index)) = state.sprite.or(layer.sprite) {
            let Some(sprite) = stage.sprites.get(group, index) else {
                continue;
            };
            (
                sprite,
                Pose {
                    offset: Vec2::ZERO,
                    flip_h: false,
                    flip_v: false,
                    blend: blend_of(state.trans.unwrap_or(layer.trans)),
                    scale: Vec2::new(1.0, 1.0),
                    alpha: 1.0,
                    angle: 0.0,
                },
            )
        } else {
            continue;
        };
        let pos = state.pos.unwrap_or(Vec2::ZERO);
        let anchor = state.start.unwrap_or(layer.start)
            + pos
            + state.offset
            + Vec2::new(
                sin_offset(state.sin_x) + cos_offset(state.cos_x),
                sin_offset(state.sin_y) + cos_offset(state.cos_y),
            );
        let mut scale = state.scale.unwrap_or(Vec2::new(1.0, 1.0));
        scale = Vec2::new(
            scale.x * layer.xscale * pose.scale.x,
            scale.y * layer.yscale * pose.scale.y,
        );
        let (anchor, scale, facing, alpha) = match transform {
            Some((pos, overlay_scale, facing, alpha)) => (
                Vec2::new(
                    pos.x + anchor.x * overlay_scale.x * facing as f32,
                    pos.y + anchor.y * overlay_scale.y,
                ),
                Vec2::new(scale.x * overlay_scale.x, scale.y * overlay_scale.y),
                facing,
                alpha,
            ),
            None => (anchor, scale, 1, 1.0),
        };
        draw_sprite(
            painter,
            atlas,
            bank_id,
            sprite,
            anchor,
            pose.offset,
            facing,
            scale,
            pose.flip_h,
            pose.flip_v,
            blend_of(state.trans.unwrap_or(layer.trans)),
            [1.0, 1.0, 1.0, pose.alpha * alpha],
            pose.angle,
        );
    }
}

pub fn draw_storyboard(
    painter: &mut Painter,
    deps: &SceneDeps,
    storyboard: &manato_formats::storyboard::Storyboard,
    player: &manato_formats::storyboard::StoryboardPlayer,
) {
    painter.clear();
    let Some(scene) = player.current(storyboard) else {
        return;
    };
    if let Some(color) = scene.clear_color {
        if scene.clear_alpha > 0 {
            painter.rect(
                Blend::Normal,
                0.0,
                0.0,
                GAME_WIDTH,
                GAME_HEIGHT,
                [
                    color[0] as f32 / 255.0,
                    color[1] as f32 / 255.0,
                    color[2] as f32 / 255.0,
                    scene.clear_alpha as f32 / 255.0,
                ],
            );
        }
    }

    if let Some(stage) = &scene.bg {
        draw_storyboard_bg(
            painter,
            deps.atlas,
            stage,
            bank::STORYBOARD_BG.saturating_add(player.scene as u8),
            player.time,
            None,
        );
    }

    let mut layers: Vec<&manato_formats::storyboard::StoryboardLayer> = scene
        .layers
        .iter()
        .filter(|layer| layer.visible_at(player.time))
        .collect();
    layers.sort_by_key(|layer| layer.layer_no);
    for layer in layers {
        let anchor = layer.position_at(player.time) + scene.layer_all_pos;
        let sprite_pose = if layer.anim >= 0 {
            storyboard.animations.get(&layer.anim).and_then(|anim| {
                let local = (player.time - layer.start_time).max(0);
                let interpolated = anim.pose_at(local)?;
                let frame = anim.frames.get(interpolated.frame)?;
                let sprite = storyboard.sprites.get(frame.group, frame.index)?;
                Some((sprite, Pose::from_pose(frame, &interpolated)))
            })
        } else if let Some((group, index)) = layer.sprite {
            storyboard.sprites.get(group, index).map(|sprite| {
                (
                    sprite,
                    Pose {
                        offset: Vec2::ZERO,
                        flip_h: false,
                        flip_v: false,
                        blend: Blend::Normal,
                        scale: Vec2::new(1.0, 1.0),
                        alpha: 1.0,
                        angle: 0.0,
                    },
                )
            })
        } else {
            None
        };
        if let Some((sprite, pose)) = sprite_pose {
            let scale = Vec2::new(layer.scale.x * pose.scale.x, layer.scale.y * pose.scale.y);
            let palfx = layer.palfx_at(player.time);
            painter.set_palfx(
                palfx_sprite_add(&palfx, pose.blend),
                palfx.color,
                palfx.hue,
                palfx_sprite_invert(&palfx, pose.blend),
            );
            draw_sprite(
                painter,
                deps.atlas,
                bank::STORYBOARD,
                sprite,
                anchor,
                pose.offset,
                layer.facing,
                scale,
                pose.flip_h,
                pose.flip_v,
                palfx_blend(&palfx, pose.blend),
                [
                    palfx.mul[0] / 256.0,
                    palfx.mul[1] / 256.0,
                    palfx.mul[2] / 256.0,
                    pose.alpha,
                ],
                pose.angle,
            );
            painter.reset_palfx();
        }
        if layer.text.is_some() {
            let text = layer.visible_text(player.time);
            if !text.is_empty() {
                let palfx = layer.palfx_at(player.time);
                let tint = [
                    layer.color[0] as f32 / 255.0 * palfx.mul[0] / 256.0,
                    layer.color[1] as f32 / 255.0 * palfx.mul[1] / 256.0,
                    layer.color[2] as f32 / 255.0 * palfx.mul[2] / 256.0,
                    layer.color[3] as f32 / 255.0,
                ];
                painter.set_palfx(palfx.add, palfx.color, palfx.hue, palfx.invert);
                if layer.font >= 0 {
                    if let Some(font) = deps.screen_fonts.get(&(layer.font as u32)) {
                        font.draw(painter, &text, anchor, layer.scale, 0, tint);
                        painter.reset_palfx();
                        continue;
                    }
                }
                let pixel_scale = layer.scale.y.max(layer.scale.x).clamp(0.4, 4.0);
                painter.text(deps.font, &text, anchor.x, anchor.y, pixel_scale, tint);
                painter.reset_palfx();
            }
        }
    }

    if let Some((color, alpha)) = player.fade_alpha(scene) {
        if alpha > 0 {
            painter.rect(
                Blend::Normal,
                0.0,
                0.0,
                GAME_WIDTH,
                GAME_HEIGHT,
                [
                    color[0] as f32 / 255.0,
                    color[1] as f32 / 255.0,
                    color[2] as f32 / 255.0,
                    alpha as f32 / 255.0,
                ],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use manato_sim::ProjectionMode;

    #[test]
    fn bg_layer_palfx_applies_sines_and_clamps() {
        let mut state =
            manato_sim::StageLayerState::from_layer(&manato_formats::stage::BgLayer::default());
        // Identity by default.
        assert_eq!(bg_layer_palfx(&state, 12.0), (1.0, 0.0, false));

        state.pal_color = 0.5;
        state.pal_sin_color = 0.75;
        state.pal_sin_color_period = 4;
        state.pal_hue = 0.25;
        state.pal_sin_hue = 0.5;
        state.pal_sin_hue_period = 8;
        state.pal_invert = true;

        // Cycle start: sine is zero, so the base values apply.
        let (color, hue, invert) = bg_layer_palfx(&state, 0.0);
        assert!((color - 0.5).abs() < 1e-5);
        assert!((hue - 0.25).abs() < 1e-5);
        assert!(invert);

        // Quarter cycle of the colour sine clamps at full saturation.
        let (color, _, _) = bg_layer_palfx(&state, 1.0);
        assert!((color - 1.0).abs() < 1e-5);

        // Three-quarter cycle clamps at zero.
        let (color, _, _) = bg_layer_palfx(&state, 3.0);
        assert!(color.abs() < 1e-5);

        // Quarter cycle of the hue sine clamps at the 0.5-turn limit.
        let (_, hue, _) = bg_layer_palfx(&state, 2.0);
        assert!((hue - 0.5).abs() < 1e-5);
    }

    #[test]
    fn depth_transform_projects_objects_and_scale() {
        let stage = manato_formats::Stage {
            z_top: 0.0,
            z_bottom: 100.0,
            z_top_scale: 1.0,
            z_bottom_scale: 0.5,
            depth_to_screen: 0.25,
            ..manato_formats::Stage::default()
        };
        let anchor = Vec2::new(200.0, 180.0);
        let scale = Vec2::new(2.0, 2.0);

        // z = 0 is the near plane with scale 1.
        let (same, same_scale) = depth_transform(anchor, scale, 0.0, 0.0, &stage, 160.0, 200.0);
        assert!((same.x - anchor.x).abs() < 1e-5);
        assert!((same.y - anchor.y).abs() < 1e-5);
        assert!((same_scale.x - 2.0).abs() < 1e-5);

        // At the far plane the object shrinks toward the pivot/ground and
        // shifts by `depthtoscreen`.
        let (far, far_scale) = depth_transform(anchor, scale, 100.0, 0.0, &stage, 160.0, 200.0);
        assert!((far.x - 180.0).abs() < 1e-5);
        assert!((far.y - 215.0).abs() < 1e-5);
        assert!((far_scale.x - 1.0).abs() < 1e-5);

        // Camera depth subtracts from the projected depth: centring the
        // camera on the object removes the depth-to-screen shift.
        let (centred, centred_scale) =
            depth_transform(anchor, scale, 100.0, 100.0, &stage, 160.0, 200.0);
        assert!((centred.x - 180.0).abs() < 1e-5);
        assert!((centred.y - 190.0).abs() < 1e-5);
        assert!((centred_scale.x - 1.0).abs() < 1e-5);
    }

    #[test]
    fn depth_projection_scales_about_camera_and_ground() {
        let anchor = Vec2::new(200.0, 180.0);
        // A depth scale of 1 with no y offset is an identity.
        let same = project_depth(anchor, 1.0, 0.0, 160.0, 200.0);
        assert!((same.x - anchor.x).abs() < 1e-5);
        assert!((same.y - anchor.y).abs() < 1e-5);

        // Scaling about the camera centre and ground line.
        let far = project_depth(anchor, 0.5, 0.0, 160.0, 200.0);
        assert!((far.x - 180.0).abs() < 1e-5);
        assert!((far.y - 190.0).abs() < 1e-5);

        // Depth-to-screen shifts Y after scaling.
        let shifted = project_depth(anchor, 1.0, 12.0, 160.0, 200.0);
        assert!((shifted.y - 192.0).abs() < 1e-5);
    }

    #[test]
    fn afterimage_palette_steps_older_ghosts() {
        use manato_sim::fighter::AfterImageState;
        let after = AfterImageState {
            time: 10,
            interval: 1,
            tick: 0,
            life: 10,
            alpha: 0.45,
            add: true,
            pal_add: [10.0, 0.0, 0.0],
            pal_mul: [0.5, 1.0, 1.0],
            pal_step_add: [5.0, 0.0, 0.0],
            pal_step_mul: [0.5, 1.0, 1.0],
            pal_post: [20.0, 0.0, 0.0],
            pal_color: 0.75,
            pal_hue: 0.25,
            pal_invert: true,
        };
        // Newest ghost: base add/mul only, no post-bright.
        let (tint, saturation, hue, invert) = afterimage_palette(Some(&after), 0.45, 0.0);
        assert!((tint[0] - (0.5 + 10.0 / 255.0)).abs() < 1e-5);
        assert!((tint[1] - 1.0).abs() < 1e-5);
        assert_eq!(tint[3], 0.45);
        assert!((saturation - 0.75).abs() < 1e-5);
        assert!((hue - 0.25).abs() < 1e-5);
        assert!(invert);

        // One step older: adds paladd, applies palmul, and adds postbright.
        let (tint, _, _, _) = afterimage_palette(Some(&after), 0.45, 1.0);
        assert!((tint[0] - (0.25 + 35.0 / 255.0)).abs() < 1e-5);

        // No active AfterImage state leaves the ghost untinted.
        let (tint, saturation, hue, invert) = afterimage_palette(None, 0.3, 2.0);
        assert_eq!(tint, [1.0, 1.0, 1.0, 0.3]);
        assert_eq!((saturation, hue, invert), (1.0, 0.0, false));
    }

    #[test]
    fn global_tint_composes_mul_add_and_alpha() {
        use manato_sim::GlobalPalFx;
        // Inactive effects leave the color untouched.
        let identity = GlobalPalFx::default();
        assert_eq!(
            global_tint([0.25, 0.5, 0.75, 0.5], &identity),
            [0.25, 0.5, 0.75, 0.5]
        );

        let pal = GlobalPalFx {
            add: [51.0, 0.0, 255.0],
            mul: [0.5, 1.0, 0.0],
            alpha: 0.5,
            time: 10,
            ..GlobalPalFx::default()
        };
        let color = global_tint([0.4, 0.4, 0.4, 0.8], &pal);
        assert!((color[0] - (0.4 * 0.5 + 51.0 / 255.0)).abs() < 1e-5);
        assert!((color[1] - 0.4).abs() < 1e-5);
        assert!((color[2] - 1.0).abs() < 1e-5);
        assert!((color[3] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn pose_interpolates_air_markers() {
        let text = "[Begin Action 0]\n\
                    0,0, 0,0, 4, , , 1,1, 0\n\
                    Interpolate Offset\nInterpolate Scale\nInterpolate Angle\n\
                    0,1, 8,16, 4, , , 2,2, 90\n";
        let anim = &manato_formats::air::parse_air_str(text)[0];
        let interpolated = anim.pose_at(2).expect("pose");
        let frame = &anim.frames[interpolated.frame];
        let pose = Pose::from_pose(frame, &interpolated);
        assert!((pose.offset.x - 4.0).abs() < 1e-5);
        assert!((pose.offset.y - 8.0).abs() < 1e-5);
        assert!((pose.scale.x - 1.5).abs() < 1e-5);
        assert!((pose.scale.y - 1.5).abs() < 1e-5);
        assert!((pose.angle - 45.0).abs() < 1e-5);
        // Sprite/flip/blend/alpha still come from the current frame.
        assert!(!pose.flip_h);
        assert_eq!(pose.blend, Blend::Normal);
    }

    #[test]
    fn pose_interpolates_air_blend_alpha() {
        let text = "[Begin Action 0]\n\
                    0,0, 0,0, 4, , a1\n\
                    Interpolate Blend\n\
                    0,1, 0,0, 4, , as64d32\n";
        let anim = &manato_formats::air::parse_air_str(text)[0];
        let interpolated = anim.pose_at(2).expect("pose");
        let frame = &anim.frames[interpolated.frame];
        let pose = Pose::from_pose(frame, &interpolated);
        assert!((pose.alpha - 159.5 / 255.0).abs() < 1e-5);
        // Blend type still comes from the current frame.
        assert_eq!(pose.blend, Blend::Add);
    }

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
    fn test_projectile(layer_no: i32, sprite_priority: i32) -> manato_sim::projectile::Projectile {
        manato_sim::projectile::Projectile {
            owner: 0,
            id: 0,
            anim: 0,
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            accel: Vec2::ZERO,
            z: 0.0,
            vel_z: 0.0,
            accel_z: 0.0,
            facing: 1,
            scale: Vec2::new(1.0, 1.0),
            angle: 0.0,
            x_shear: 0.0,
            projection: ProjectionMode::Orthographic,
            focal_length: 1024.0,
            time: 0,
            remove_time: -1,
            anim_loop_time: 0,
            remove_on_hits_exhausted: true,
            priority: 0,
            priority_points: 0,
            shadow: [0; 3],
            reflection: 0,
            miss_time: 0,
            hit_anim: -1,
            rem_anim: -1,
            cancel_anim: -1,
            layer_no,
            sprite_priority,
            edge_bound: 40.0,
            stage_bound: 40.0,
            height_bound: (-240.0, 1.0),
            depth_bound: f32::MAX,
            out_of_bounds: false,
            status: manato_sim::projectile::ProjectileStatus::Active,
            removal_ticks: 0,
            rem_velocity: Vec2::ZERO,
            rem_velocity_z: 0.0,
            hits: 1,
            hits_max: 1,
            hitdef: manato_sim::hitdef::HitDef::default(),
        }
    }

    #[test]
    fn projectile_draw_order_splits_layers_and_sorts_by_priority() {
        let projectiles = [
            test_projectile(0, 5),
            test_projectile(1, 1),
            test_projectile(0, 2),
            test_projectile(1, 9),
        ];
        assert_eq!(projectile_draw_order(&projectiles, false), vec![2, 0]);
        assert_eq!(projectile_draw_order(&projectiles, true), vec![1, 3]);
        // Equal priorities keep insertion order.
        let equal = [
            test_projectile(0, 3),
            test_projectile(0, 3),
            test_projectile(0, 1),
        ];
        assert_eq!(projectile_draw_order(&equal, false), vec![2, 0, 1]);
    }
}

#[cfg(test)]
mod palfx_tests {
    use super::*;
    use manato_formats::storyboard::PalFxSample;

    #[test]
    fn invertblend_swaps_add_sub_passes_only() {
        let identity = PalFxSample::identity();
        assert_eq!(palfx_blend(&identity, Blend::Add), Blend::Add);
        let swapped = PalFxSample {
            add: [10.0, 20.0, 30.0],
            invert: true,
            invert_blend: 1,
            ..identity
        };
        assert_eq!(palfx_blend(&swapped, Blend::Add), Blend::Sub);
        assert_eq!(palfx_blend(&swapped, Blend::Sub), Blend::Add);
        assert_eq!(palfx_blend(&swapped, Blend::Normal), Blend::Normal);
        assert_eq!(
            palfx_sprite_add(&swapped, Blend::Sub),
            [-10.0, -20.0, -30.0]
        );
        assert_eq!(
            palfx_sprite_add(&swapped, Blend::Normal),
            [10.0, 20.0, 30.0]
        );
        assert!(palfx_sprite_invert(&swapped, Blend::Sub));
    }

    #[test]
    fn invertblend_three_disables_shader_invert_on_sub_passes() {
        let sample = PalFxSample {
            invert: true,
            invert_blend: 3,
            ..PalFxSample::identity()
        };
        assert!(!palfx_sprite_invert(&sample, Blend::Sub));
        assert!(palfx_sprite_invert(&sample, Blend::Normal));
    }
}
