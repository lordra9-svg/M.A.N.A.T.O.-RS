//! Title and select screen rendering (`docs/spec/render.md` §7–§8).
//!
//! These screens are front-end only: they never touch simulation state and
//! are driven by an app-owned frame counter.

use manato_core::{Vec2, GAME_HEIGHT, GAME_WIDTH};
use manato_formats::screenpack::{Layer, ScreenBg, Screenpack};

use crate::atlas::bank;
use crate::draw::{Blend, Painter, TexRef};
use crate::fightscreen::{draw_element, draw_layer, draw_text, ScreenMap};

pub type FrontendDeps<'a> = crate::fightscreen::ScreenDeps<'a>;

/// App-owned title screen state.
#[derive(Debug, Clone, Default)]
pub struct TitleView {
    pub selected: usize,
    pub frame: u32,
}

/// One visible roster slot.
#[derive(Debug, Clone, Default)]
pub struct SelectEntry {
    /// Short name drawn under the grid cell.
    pub name: String,
    /// Full display name drawn beside the grid.
    pub display_name: String,
    pub random: bool,
}

/// App-owned select screen state.
#[derive(Debug, Clone, Default)]
pub struct SelectView {
    pub p1: usize,
    pub p2: usize,
    pub page: i32,
    pub p1_done: bool,
    pub p2_done: bool,
    pub frame: u32,
    /// Remaining select timer ticks; `None` hides the counter.
    pub timer: Option<i32>,
    pub names: Vec<SelectEntry>,
}

#[allow(clippy::too_many_arguments)]
fn draw_background(
    painter: &mut Painter,
    deps: &FrontendDeps,
    bg: &ScreenBg,
    map: &ScreenMap,
    states: &[manato_sim::StageLayerState],
    time: i32,
) {
    let mut order: Vec<(usize, &manato_formats::stage::BgLayer)> = bg
        .layers
        .iter()
        .enumerate()
        .filter(|(index, layer)| {
            states
                .get(*index)
                .map(|state| state.visible && state.enabled)
                .unwrap_or(layer.visible && layer.enabled)
        })
        .collect();
    order.sort_by_key(|(_, layer)| layer.layer_no);
    for (index, layer) in order {
        let state = states.get(index);
        let motion = |values: [f32; 3], cos: bool| {
            if values[1].abs() <= f32::EPSILON {
                return 0.0;
            }
            let angle = std::f32::consts::TAU * time as f32 / values[1] + values[2].to_radians();
            if cos {
                values[0] * angle.cos()
            } else {
                values[0] * angle.sin()
            }
        };
        let (sin_x, sin_y, cos_x, cos_y) = match state {
            Some(state) => (state.sin_x, state.sin_y, state.cos_x, state.cos_y),
            None => (layer.sin_x, layer.sin_y, layer.cos_x, layer.cos_y),
        };
        let offset = state.map(|state| state.offset).unwrap_or(Vec2::ZERO);
        let start = map.point(
            layer.start
                + offset
                + Vec2::new(
                    motion(sin_x, false) + motion(cos_x, true),
                    motion(sin_y, false) + motion(cos_y, true),
                ),
        );

        let action = state.and_then(|state| state.action).or(layer.action);
        let (group, index, frame_offset, frame_scale, flip_h, flip_v, alpha, blend) =
            if let Some(action) = action {
                let Some(animation) = bg.animations.get(&action) else {
                    continue;
                };
                let Some((frame_idx, _)) = animation.frame_at(time) else {
                    continue;
                };
                let Some(frame) = animation.frames.get(frame_idx) else {
                    continue;
                };
                (
                    frame.group,
                    frame.index,
                    frame.offset,
                    Vec2::new(frame.scale.x.abs(), frame.scale.y.abs()),
                    frame.flip_h,
                    frame.flip_v,
                    frame.alpha(),
                    match frame.blend {
                        manato_formats::air::BlendMode::Normal => Blend::Normal,
                        manato_formats::air::BlendMode::Add => Blend::Add,
                        _ => Blend::Sub,
                    },
                )
            } else {
                let Some((group, index)) = layer.sprite else {
                    continue;
                };
                (
                    group,
                    index,
                    Vec2::ZERO,
                    Vec2::new(1.0, 1.0),
                    false,
                    false,
                    1.0,
                    Blend::Normal,
                )
            };
        let Some(sprite) = bg.sprites.get(group, index) else {
            continue;
        };
        let Some(atlas_sprite) = deps.atlas.get(bank::SCREENPACK, group, index) else {
            continue;
        };
        let origin = start + frame_offset;
        let w = sprite.width as f32 * layer.xscale * frame_scale.x * map.scale;
        let h = sprite.height as f32 * layer.yscale * frame_scale.y * map.scale;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let step_x = if layer.tile.0 != 0 {
            (w + layer.tile_spacing.0 as f32 * map.scale).max(1.0)
        } else {
            w + 1.0
        };
        let step_y = if layer.tile.1 != 0 {
            (h + layer.tile_spacing.1 as f32 * map.scale).max(1.0)
        } else {
            h + 1.0
        };
        let mut y = origin.y;
        while y < GAME_HEIGHT {
            let mut x = origin.x;
            while x < GAME_WIDTH {
                painter.quad(
                    TexRef::Atlas(atlas_sprite.page),
                    blend,
                    [x, y, x + w, y + h],
                    atlas_sprite.uv,
                    flip_h,
                    flip_v,
                    [1.0, 1.0, 1.0, alpha],
                );
                if layer.tile.0 == 0 {
                    break;
                }
                x += step_x;
            }
            if layer.tile.1 == 0 {
                break;
            }
            y += step_y;
        }
    }
}

fn menu_items(screenpack: &Screenpack) -> Vec<&manato_formats::screenpack::MenuItemDef> {
    let order = ["arcade", "versus", "training", "options", "quit"];
    let mut items: Vec<_> = screenpack
        .title
        .items
        .iter()
        .filter(|item| order.contains(&item.key.as_str()))
        .collect();
    items.sort_by_key(|item| {
        order
            .iter()
            .position(|key| *key == item.key)
            .unwrap_or(usize::MAX)
    });
    items
}

/// Paints the title screen over its background.
pub fn draw_title(
    painter: &mut Painter,
    deps: &FrontendDeps,
    screenpack: &Screenpack,
    states: &[manato_sim::StageLayerState],
    view: &TitleView,
) {
    let map = ScreenMap::new(screenpack);
    if let Some(bg) = &screenpack.title_bg {
        draw_background(painter, deps, bg, &map, states, view.frame as i32);
    }
    let title = &screenpack.title;
    let fight = &screenpack.fight;
    if !title.title.is_empty() {
        draw_element(painter, deps, &title.title, fight, &map, Vec2::ZERO);
    }

    let items = menu_items(screenpack);
    for (index, item) in items.iter().enumerate() {
        let pos = title.menu_pos + title.item_offset + title.item_spacing * index as f32;
        let selected = index == view.selected % items.len().max(1);
        let color = if selected {
            title.active_color
        } else {
            title.item_color
        };
        let color = [
            color[0] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[2] as f32 / 255.0,
            color[3] as f32 / 255.0,
        ];
        if selected {
            if let Some((group, index)) = title.active_sprite {
                let layer = Layer {
                    sprite: Some((group, index)),
                    ..Layer::default()
                };
                draw_layer(
                    painter,
                    deps,
                    &layer,
                    fight,
                    &map,
                    pos,
                    view.frame as i32,
                    color,
                );
            }
        }
        let def = manato_formats::screenpack::TextDef {
            font: if selected {
                title.active_font
            } else {
                title.item_font
            },
            text: item.label.clone(),
            offset: Vec2::ZERO,
            scale: if selected {
                title.active_scale
            } else {
                title.item_scale
            },
            align: 1,
            color: [
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
                255,
            ],
        };
        draw_text(painter, deps, &def, &item.label, &map, pos, 1, [1.0; 4]);
    }
}

fn cell_position(screenpack: &Screenpack, index: usize) -> Vec2 {
    let select = &screenpack.select;
    let columns = select.columns.max(1) as usize;
    let col = index % columns;
    let row = index / columns;
    select.pos
        + Vec2::new(
            col as f32 * (select.cell_size.x + select.cell_spacing.x),
            row as f32 * (select.cell_size.y + select.cell_spacing.y),
        )
}

/// Paints the select screen over its background.
pub fn draw_select(
    painter: &mut Painter,
    deps: &FrontendDeps,
    screenpack: &Screenpack,
    states: &[manato_sim::StageLayerState],
    view: &SelectView,
) {
    let map = ScreenMap::new(screenpack);
    if let Some(bg) = &screenpack.select_bg {
        draw_background(painter, deps, bg, &map, states, view.frame as i32);
    }
    let select = &screenpack.select;
    let fight = &screenpack.fight;
    let columns = select.columns.max(1) as usize;
    let per_page = (select.rows.max(1) as usize) * columns;
    let page_start = (view.page.max(0) as usize) * per_page;

    for slot in 0..per_page {
        let index = page_start + slot;
        let position = cell_position(screenpack, slot);
        if let Some(entry) = view.names.get(index) {
            if select.cell_bg.sprite.is_some() {
                draw_layer(
                    painter,
                    deps,
                    &select.cell_bg,
                    fight,
                    &map,
                    position,
                    view.frame as i32,
                    [1.0; 4],
                );
            }
            if entry.random && select.cell_random.sprite.is_some() {
                draw_layer(
                    painter,
                    deps,
                    &select.cell_random,
                    fight,
                    &map,
                    position,
                    view.frame as i32,
                    [1.0; 4],
                );
            }
            draw_select_face(
                painter,
                deps,
                screenpack,
                map,
                index,
                &select.cell_face,
                position,
                view.frame as i32,
            );
            let name = &select.cell_name;
            if !name.text.is_empty() || !entry.name.is_empty() {
                let text = if name.text.is_empty() {
                    entry.name.chars().take(9).collect::<String>()
                } else {
                    name.text.clone()
                };
                draw_text(
                    painter, deps, name, &text, &map, position, name.align, [1.0; 4],
                );
            }
        } else if select.show_empty_boxes {
            if select.cell_bg.sprite.is_some() {
                draw_layer(
                    painter,
                    deps,
                    &select.cell_bg,
                    fight,
                    &map,
                    position,
                    view.frame as i32,
                    [0.5, 0.5, 0.5, 0.5],
                );
            }
        }
    }

    // Player cursors, one per side.
    for (side, cell) in [(0usize, view.p1), (1usize, view.p2)] {
        if view.names.is_empty() {
            continue;
        }
        let slot = cell.saturating_sub(page_start) % per_page.max(1);
        let position = cell_position(screenpack, slot);
        let cursor = &select.players[side];
        let chosen = if (side == 0 && view.p1_done) || (side == 1 && view.p2_done) {
            &cursor.cursor_done
        } else {
            &cursor.cursor
        };
        if let Some((group, index)) = chosen.sprite {
            let layer = Layer {
                sprite: Some((group, index)),
                offset: chosen.offset,
                ..Layer::default()
            };
            draw_layer(
                painter,
                deps,
                &layer,
                fight,
                &map,
                position,
                view.frame as i32,
                [1.0; 4],
            );
        }
    }

    // Large portraits for both players.
    for (side, cell) in [(0usize, view.p1), (1usize, view.p2)] {
        let player = &select.players[side];
        let mut layer = player.face.clone();
        layer.sprite = layer.sprite.or(player.face_spr);
        draw_select_face(
            painter,
            deps,
            screenpack,
            map,
            cell,
            &layer,
            Vec2::ZERO,
            view.frame as i32,
        );
    }

    // Large names for both players.
    for (side, cell) in [(0usize, view.p1), (1usize, view.p2)] {
        if let Some(entry) = view.names.get(cell) {
            let name = &select.players[side].name;
            if !name.text.is_empty() || !entry.name.is_empty() {
                let text = if name.text.is_empty() {
                    entry.display_name.clone()
                } else {
                    name.text.clone()
                };
                draw_text(
                    painter,
                    deps,
                    name,
                    &text,
                    &map,
                    Vec2::ZERO,
                    name.align,
                    [1.0; 4],
                );
            }
        }
    }

    // Stage preview element, if declared.
    if !select.stage.is_empty() {
        draw_element(painter, deps, &select.stage, fight, &map, Vec2::ZERO);
    }

    // Select countdown, when enabled.
    if let Some(ticks) = view.timer {
        if select.timer_count > 0 {
            let seconds = (ticks + 59) / 60;
            let text = if select.timer.counter.text.is_empty() {
                format!("{seconds:02}")
            } else {
                select
                    .timer
                    .counter
                    .text
                    .replace("%d", &seconds.to_string())
            };
            draw_text(
                painter,
                deps,
                &select.timer.counter,
                &text,
                &map,
                select.timer.pos,
                select.timer.counter.align,
                [1.0; 4],
            );
        }
    }
}

fn draw_select_face(
    painter: &mut Painter,
    deps: &FrontendDeps,
    screenpack: &Screenpack,
    map: ScreenMap,
    roster_index: usize,
    layer: &Layer,
    position: Vec2,
    time: i32,
) {
    let Some((group, index)) = layer.sprite else {
        return;
    };
    let Some(sprite) = deps
        .atlas
        .get(bank::CHARACTER_BASE + roster_index as u8, group, index)
        .or_else(|| deps.atlas.get(bank::SCREENPACK, group, index))
    else {
        return;
    };
    let origin = map.point(position + layer.offset);
    let w = sprite.width * layer.scale.x * map.scale;
    let h = sprite.height * layer.scale.y * map.scale;
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    painter.quad(
        TexRef::Atlas(sprite.page),
        Blend::Normal,
        [origin.x, origin.y, origin.x + w, origin.y + h],
        sprite.uv,
        false,
        false,
        [1.0; 4],
    );
    let _ = (screenpack, time);
}

/// App-owned options overlay state (`docs/spec/render.md` §9.2).
#[derive(Debug, Clone, Default)]
pub struct OptionsView {
    pub selected: usize,
    pub frame: u32,
    pub items: Vec<(String, String)>,
}

/// Paints the engine-owned options list over the title background.
pub fn draw_options(
    painter: &mut Painter,
    deps: &FrontendDeps,
    screenpack: &Screenpack,
    states: &[manato_sim::StageLayerState],
    view: &OptionsView,
) {
    let map = ScreenMap::new(screenpack);
    if let Some(bg) = &screenpack.title_bg {
        draw_background(painter, deps, bg, &map, states, view.frame as i32);
    }
    painter.rect(
        Blend::Normal,
        0.0,
        0.0,
        GAME_WIDTH,
        GAME_HEIGHT,
        [0.02, 0.03, 0.06, 0.82],
    );
    let fight = &screenpack.fight;
    for (index, (label, value)) in view.items.iter().enumerate() {
        let selected = index == view.selected;
        let y = 64.0 + index as f32 * 14.0;
        let color = if selected {
            [1.0, 0.84, 0.36, 1.0]
        } else {
            [0.86, 0.88, 0.94, 1.0]
        };
        let def = manato_formats::screenpack::TextDef {
            font: 0,
            text: label.clone(),
            offset: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            align: 2,
            color: [255, 255, 255, 255],
        };
        draw_text(
            painter,
            deps,
            &def,
            label,
            &map,
            Vec2::new(130.0, y),
            2,
            color,
        );
        draw_text(
            painter,
            deps,
            &def,
            value,
            &map,
            Vec2::new(140.0, y),
            0,
            color,
        );
        let _ = fight;
    }
    let title = manato_formats::screenpack::TextDef {
        font: 0,
        text: "OPTIONS".into(),
        offset: Vec2::ZERO,
        scale: Vec2::new(2.0, 2.0),
        align: 1,
        color: [255, 255, 255, 255],
    };
    draw_text(
        painter,
        deps,
        &title,
        "OPTIONS",
        &map,
        Vec2::new(160.0, 32.0),
        1,
        [1.0, 0.9, 0.6, 1.0],
    );
}
