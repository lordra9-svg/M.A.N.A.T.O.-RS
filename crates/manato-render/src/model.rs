//! Minimal static glTF model rendering with base-color textures and node
//! animation.
//!
//! Triangle meshes are transformed by their glTF node hierarchy, optionally
//! animated by the first glTF animation (translation/rotation/scale, step or
//! linear), and baked into screen space. Skinning, morph targets, PBR
//! lighting, and environments are not modeled.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt as _;

use crate::{GAME_HEIGHT, GAME_WIDTH};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModelVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub joints: [u16; 4],
    pub weights: [f32; 4],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct ModelTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ModelSkin {
    joints: Vec<usize>,
    inverse_bind: Vec<Mat4>,
}

#[derive(Debug, Clone)]
struct PrefilteredEnvironment {
    levels: Vec<ModelTexture>,
}

impl PrefilteredEnvironment {
    fn new(base: ModelTexture) -> Self {
        const LEVELS: usize = 5;
        let mut levels = vec![base.clone()];
        for level in 1..LEVELS {
            let roughness = level as f32 / (LEVELS - 1) as f32;
            let width = (base.width >> level).max(4);
            let height = (base.height >> level).max(2);
            let mut rgba = vec![0u8; (width * height * 4) as usize];
            for y in 0..height {
                for x in 0..width {
                    let normal = equirect_direction(x, y, width, height);
                    let color = convolve_environment(&base, normal, roughness, 64);
                    let offset = ((y * width + x) * 4) as usize;
                    rgba[offset] = (color[0] * 255.0).clamp(0.0, 255.0) as u8;
                    rgba[offset + 1] = (color[1] * 255.0).clamp(0.0, 255.0) as u8;
                    rgba[offset + 2] = (color[2] * 255.0).clamp(0.0, 255.0) as u8;
                    rgba[offset + 3] = 255;
                }
            }
            levels.push(ModelTexture {
                width,
                height,
                rgba,
            });
        }
        Self { levels }
    }

    fn base(&self) -> &ModelTexture {
        &self.levels[0]
    }

    fn sample(&self, direction: Vec3, roughness: f32) -> [f32; 3] {
        let level = ((roughness.clamp(0.0, 1.0) * (self.levels.len() - 1) as f32).round() as usize)
            .min(self.levels.len() - 1);
        environment_color(&self.levels[level], direction)
    }
}

fn equirect_direction(x: u32, y: u32, width: u32, height: u32) -> Vec3 {
    let u = (x as f32 + 0.5) / width as f32;
    let v = (y as f32 + 0.5) / height as f32;
    let phi = (u - 0.5) * std::f32::consts::TAU;
    let theta = v * std::f32::consts::PI;
    Vec3::new(
        theta.sin() * phi.sin(),
        theta.cos(),
        theta.sin() * phi.cos(),
    )
}

/// GGX importance-sampled prefilter around `normal`.
fn convolve_environment(
    base: &ModelTexture,
    normal: Vec3,
    roughness: f32,
    samples: usize,
) -> [f32; 3] {
    let normal = normal.normalize_or_zero();
    if normal.length_squared() == 0.0 {
        return [0.0; 3];
    }
    let a = (roughness * roughness).max(0.001);
    let up = if normal.y.abs() < 0.999 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let tangent = up.cross(normal).normalize_or_zero();
    let bitangent = normal.cross(tangent);
    let mut sum = Vec3::ZERO;
    let mut weight = 0.0;
    for index in 0..samples {
        let xi_x = (index as f32 + 0.5) / samples as f32;
        let xi_y = ((index as u32).reverse_bits() as f32) * 2.328_306_4e-10;
        let phi = std::f32::consts::TAU * xi_x;
        let cos_theta = ((1.0 - xi_y) / (1.0 + (a * a - 1.0) * xi_y))
            .max(0.0)
            .sqrt();
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let half = tangent * (sin_theta * phi.cos())
            + bitangent * (sin_theta * phi.sin())
            + normal * cos_theta;
        let light = 2.0 * normal.dot(half) * half - normal;
        let n_dot_l = normal.dot(light);
        if n_dot_l <= 0.0 {
            continue;
        }
        sum += Vec3::from_array(environment_color(base, light)) * n_dot_l;
        weight += n_dot_l;
    }
    if weight <= 0.0 {
        [0.0; 3]
    } else {
        (sum / weight).to_array()
    }
}

/// CPU BRDF LUT (`32x32`) for split-sum specular integration.
fn brdf_lut(n_dot_v: f32, roughness: f32) -> (f32, f32) {
    const SIZE: usize = 32;
    static LUT: std::sync::OnceLock<Vec<(f32, f32)>> = std::sync::OnceLock::new();
    let table = LUT.get_or_init(|| {
        let mut table = Vec::with_capacity(SIZE * SIZE);
        for y in 0..SIZE {
            let roughness = (y as f32 + 0.5) / SIZE as f32;
            for x in 0..SIZE {
                let n_dot_v = (x as f32 + 0.5) / SIZE as f32;
                table.push(integrate_brdf(n_dot_v, roughness, 32));
            }
        }
        table
    });
    let x = ((n_dot_v.clamp(0.0, 1.0) * SIZE as f32) as usize).min(SIZE - 1);
    let y = ((roughness.clamp(0.0, 1.0) * SIZE as f32) as usize).min(SIZE - 1);
    table[y * SIZE + x]
}

fn integrate_brdf(n_dot_v: f32, roughness: f32, samples: usize) -> (f32, f32) {
    let n_dot_v = n_dot_v.clamp(0.001, 1.0);
    let roughness = roughness.clamp(0.04, 1.0);
    let a = roughness * roughness;
    let v = Vec3::new((1.0 - n_dot_v * n_dot_v).max(0.0).sqrt(), 0.0, n_dot_v);
    let mut scale = 0.0;
    let mut bias = 0.0;
    for index in 0..samples {
        let xi_x = (index as f32 + 0.5) / samples as f32;
        let bits = (index as u32).reverse_bits();
        let xi_y = bits as f32 * 2.328_306_4e-10;
        let phi = std::f32::consts::TAU * xi_x;
        let cos_theta = ((1.0 - xi_y) / (1.0 + (a * a - 1.0) * xi_y))
            .max(0.0)
            .sqrt();
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let half = Vec3::new(sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta);
        let light = 2.0 * v.dot(half) * half - v;
        if light.z <= 0.0 {
            continue;
        }
        let n_dot_l = light.z;
        let n_dot_h = half.z;
        let v_dot_h = v.dot(half);
        let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
        let geometry =
            (n_dot_l / (n_dot_l * (1.0 - k) + k)) * (n_dot_v / (n_dot_v * (1.0 - k) + k));
        let visibility = geometry * v_dot_h / (n_dot_h * n_dot_v).max(1e-6);
        let fresnel = (1.0 - v_dot_h).clamp(0.0, 1.0).powi(5);
        scale += (1.0 - fresnel) * visibility;
        bias += fresnel * visibility;
    }
    (scale / samples as f32, bias / samples as f32)
}

#[derive(Debug, Clone, Copy)]
struct IblAmbient {
    diffuse: [f32; 3],
    specular: [f32; 3],
    scale: f32,
    bias: f32,
}

#[derive(Debug, Clone)]
pub struct ModelDraw {
    /// Node-local vertex positions plus UV/color.
    pub vertices: Vec<ModelVertex>,
    pub node: usize,
    pub texture: Option<ModelTexture>,
    skin: Option<ModelSkin>,
    /// Morph-target position deltas, one list per target and vertex.
    targets: Vec<Vec<[f32; 3]>>,
    /// Default morph weights (node override or mesh default).
    weights: Vec<f32>,
    metallic: f32,
    roughness: f32,
    normal_map: Option<ModelTexture>,
    normal_scale: f32,
    emissive_map: Option<ModelTexture>,
    emissive_factor: [f32; 3],
    orm_map: Option<ModelTexture>,
}

#[derive(Debug, Clone)]
struct WeightTrack {
    node: usize,
    keys: Vec<(f32, Vec<f32>)>,
}

#[derive(Debug, Clone)]
struct ModelNode {
    parent: Option<usize>,
    base_translation: Vec3,
    base_rotation: Quat,
    base_scale: Vec3,
}

#[derive(Debug, Clone, Copy)]
enum Property {
    Translation,
    Rotation,
    Scale,
}

#[derive(Debug, Clone)]
struct Track {
    node: usize,
    property: Property,
    keys: Vec<(f32, [f32; 4])>,
}

#[derive(Debug, Clone)]
struct ModelAnimation {
    name: Option<String>,
    tracks: Vec<Track>,
    weight_tracks: Vec<WeightTrack>,
}

#[derive(Debug, Clone)]
struct Placement {
    center: Vec3,
    min_y: f32,
    scale: f32,
    ground: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ModelAsset {
    pub draws: Vec<ModelDraw>,
    nodes: Vec<ModelNode>,
    animations: Vec<ModelAnimation>,
    clip: usize,
    environment: Option<PrefilteredEnvironment>,
    world0: Vec<Mat4>,
    placement: Option<Placement>,
}

impl ModelAsset {
    /// Vertex data for every draw at `time` seconds into the first animation.
    /// Selects an animation clip by index or case-insensitive name.
    pub fn select_clip(&mut self, selector: &str) -> bool {
        if let Ok(index) = selector.trim().parse::<usize>() {
            if index < self.animations.len() {
                self.clip = index;
                return true;
            }
        }
        let selector = selector.trim().to_ascii_lowercase();
        if let Some((index, _)) = self.animations.iter().enumerate().find(|(_, animation)| {
            animation
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&selector))
        }) {
            self.clip = index;
            return true;
        }
        false
    }

    pub fn clip_names(&self) -> Vec<&str> {
        self.animations
            .iter()
            .map(|animation| animation.name.as_deref().unwrap_or(""))
            .collect()
    }

    /// Loads an equirectangular environment map used for image-based ambient.
    pub fn set_environment(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let image = image::open(path)?.to_rgba8();
        self.environment = Some(PrefilteredEnvironment::new(ModelTexture {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        }));
        Ok(())
    }

    pub fn environment(&self) -> Option<&ModelTexture> {
        self.environment.as_ref().map(PrefilteredEnvironment::base)
    }

    pub fn sample(
        &self,
        time: f32,
        node_overrides: &std::collections::HashMap<i32, (f32, f32, f32)>,
    ) -> Vec<Vec<ModelVertex>> {
        let worlds = self.world_transforms(time);
        let Some(placement) = &self.placement else {
            return Vec::new();
        };
        self.draws
            .iter()
            .map(|draw| {
                let world = worlds.get(draw.node).copied().unwrap_or(Mat4::IDENTITY);
                let base = self
                    .world0
                    .get(draw.node)
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                let delta = world * base.inverse();
                draw.vertices
                    .iter()
                    .enumerate()
                    .map(|(vertex_index, vertex)| {
                        let mut local = Vec3::from_array(vertex.pos);
                        if !draw.targets.is_empty() {
                            let weights = self.morph_weights(draw, time);
                            for (target_index, target) in draw.targets.iter().enumerate() {
                                let weight = weights.get(target_index).copied().unwrap_or(0.0);
                                if weight == 0.0 {
                                    continue;
                                }
                                if let Some(delta) = target.get(vertex_index) {
                                    local += weight * Vec3::from_array(*delta);
                                }
                            }
                        }
                        let transformed = match &draw.skin {
                            Some(skin) => {
                                let mut sum = Vec3::ZERO;
                                for slot in 0..4 {
                                    let weight = vertex.weights[slot];
                                    if weight <= 0.0 {
                                        continue;
                                    }
                                    let joint = vertex.joints[slot] as usize;
                                    let Some(&joint_node) = skin.joints.get(joint) else {
                                        continue;
                                    };
                                    let Some(bind) = skin.inverse_bind.get(joint) else {
                                        continue;
                                    };
                                    let joint_world =
                                        worlds.get(joint_node).copied().unwrap_or(Mat4::IDENTITY);
                                    sum += weight * (joint_world * *bind).transform_point3(local);
                                }
                                sum
                            }
                            None => delta.transform_point3(local),
                        };
                        let mut out = *vertex;
                        out.pos = [
                            GAME_WIDTH as f32 * 0.5
                                + (transformed.x - placement.center.x) * placement.scale,
                            placement.ground - (transformed.y - placement.min_y) * placement.scale,
                            0.98,
                        ];
                        let node_id = draw.node as i32;
                        if let Some((z, xscale, yscale)) = node_overrides
                            .get(&node_id)
                            .or_else(|| node_overrides.get(&-1))
                        {
                            let center = [GAME_WIDTH as f32 * 0.5, placement.ground];
                            out.pos[0] = center[0] + (out.pos[0] - center[0]) * xscale;
                            out.pos[1] = center[1] + (out.pos[1] - center[1]) * yscale;
                            out.pos[2] = (0.98 - z * 0.001).clamp(0.05, 0.98);
                        }
                        let normal = Vec3::from_array(vertex.normal);
                        let world_normal = match &draw.skin {
                            Some(skin) => {
                                let mut sum = Vec3::ZERO;
                                for slot in 0..4 {
                                    let weight = vertex.weights[slot];
                                    if weight <= 0.0 {
                                        continue;
                                    }
                                    let joint = vertex.joints[slot] as usize;
                                    let (Some(&joint_node), Some(bind)) =
                                        (skin.joints.get(joint), skin.inverse_bind.get(joint))
                                    else {
                                        continue;
                                    };
                                    let joint_world =
                                        worlds.get(joint_node).copied().unwrap_or(Mat4::IDENTITY);
                                    sum += weight * (joint_world * *bind).transform_vector3(normal);
                                }
                                sum
                            }
                            None => delta.transform_vector3(normal),
                        }
                        .normalize_or_zero();
                        let mut normal = world_normal;
                        if let Some(map) = &draw.normal_map {
                            let texel = sample_texture(map, vertex.uv);
                            let nx = texel[0] * 2.0 - 1.0;
                            let ny = texel[1] * 2.0 - 1.0;
                            normal = (normal + Vec3::new(nx, ny, 0.0) * draw.normal_scale)
                                .normalize_or_zero();
                        }
                        let (metallic, roughness, occlusion) = if let Some(map) = &draw.orm_map {
                            let texel = sample_texture(map, vertex.uv);
                            (
                                (draw.metallic * texel[2]).clamp(0.0, 1.0),
                                (draw.roughness * texel[1]).clamp(0.0, 1.0),
                                texel[0],
                            )
                        } else {
                            (draw.metallic, draw.roughness, 1.0)
                        };
                        let environment = self.environment.as_ref().map(|environment| {
                            let view = Vec3::Z;
                            let reflect =
                                (2.0 * normal.dot(view) * normal - view).normalize_or_zero();
                            let (scale, bias) = brdf_lut(normal.dot(view).max(0.001), roughness);
                            IblAmbient {
                                diffuse: environment_color(environment.base(), normal),
                                specular: environment.sample(reflect, roughness),
                                scale,
                                bias,
                            }
                        });
                        out.color = shade_color(
                            normal,
                            out.color,
                            metallic,
                            roughness,
                            occlusion,
                            environment,
                        );
                        if let Some(map) = &draw.emissive_map {
                            let texel = sample_texture(map, vertex.uv);
                            for channel in 0..3 {
                                out.color[channel] = (out.color[channel]
                                    + texel[channel] * draw.emissive_factor[channel])
                                    .clamp(0.0, 1.0);
                            }
                        }
                        out
                    })
                    .collect()
            })
            .collect()
    }

    fn world_transforms(&self, time: f32) -> Vec<Mat4> {
        let animated = self.animated_local_transforms(time);
        let mut worlds = vec![Mat4::IDENTITY; self.nodes.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            let local = animated.get(index).copied().unwrap_or_else(|| {
                Mat4::from_scale_rotation_translation(
                    node.base_scale,
                    node.base_rotation,
                    node.base_translation,
                )
            });
            worlds[index] = match node.parent {
                Some(parent) => worlds[parent] * local,
                None => local,
            };
        }
        worlds
    }

    fn animated_local_transforms(&self, time: f32) -> Vec<Mat4> {
        let mut transforms: Vec<Option<Mat4>> = vec![None; self.nodes.len()];
        let Some(animation) = self.animations.get(self.clip) else {
            return Vec::new();
        };
        for track in &animation.tracks {
            let Some(node) = self.nodes.get(track.node) else {
                continue;
            };
            let mut translation = node.base_translation;
            let mut rotation = node.base_rotation;
            let mut scale = node.base_scale;
            let Some(value) = sample_track(&track.keys, time) else {
                continue;
            };
            match track.property {
                Property::Translation => translation = Vec3::new(value[0], value[1], value[2]),
                Property::Rotation => {
                    rotation = Quat::from_xyzw(value[0], value[1], value[2], value[3]).normalize()
                }
                Property::Scale => scale = Vec3::new(value[0], value[1], value[2]),
            }
            transforms[track.node] = Some(Mat4::from_scale_rotation_translation(
                scale,
                rotation,
                translation,
            ));
        }
        transforms
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                value.unwrap_or_else(|| {
                    let node = &self.nodes[index];
                    Mat4::from_scale_rotation_translation(
                        node.base_scale,
                        node.base_rotation,
                        node.base_translation,
                    )
                })
            })
            .collect()
    }
}

impl ModelAsset {
    fn morph_weights(&self, draw: &ModelDraw, time: f32) -> Vec<f32> {
        if let Some(animation) = self.animations.get(self.clip) {
            if let Some(track) = animation
                .weight_tracks
                .iter()
                .find(|track| track.node == draw.node)
            {
                if let Some(value) = sample_weight_track(&track.keys, time) {
                    return value;
                }
            }
        }
        draw.weights.clone()
    }
}

fn sample_weight_track(keys: &[(f32, Vec<f32>)], time: f32) -> Option<Vec<f32>> {
    if keys.is_empty() {
        return None;
    }
    if time <= keys[0].0 || keys.len() == 1 {
        return Some(keys[0].1.clone());
    }
    for pair in keys.windows(2) {
        let (start, a) = &pair[0];
        let (end, b) = &pair[1];
        if time < *end {
            let t = if *end > *start {
                (time - *start) / (*end - *start)
            } else {
                0.0
            };
            let len = a.len().min(b.len());
            return Some(
                (0..len)
                    .map(|index| a[index] + (b[index] - a[index]) * t)
                    .collect(),
            );
        }
    }
    keys.last().map(|(_, value)| value.clone())
}

fn sample_texture(texture: &ModelTexture, uv: [f32; 2]) -> [f32; 4] {
    let x = (uv[0].rem_euclid(1.0) * texture.width as f32) as u32;
    let y = (uv[1].rem_euclid(1.0) * texture.height as f32) as u32;
    let x = x.min(texture.width.saturating_sub(1));
    let y = y.min(texture.height.saturating_sub(1));
    let offset = ((y * texture.width + x) * 4) as usize;
    let px = &texture.rgba[offset..offset + 4];
    [
        px[0] as f32 / 255.0,
        px[1] as f32 / 255.0,
        px[2] as f32 / 255.0,
        px[3] as f32 / 255.0,
    ]
}

/// Simple directional + ambient model lighting. Metallic surfaces reduce the
/// diffuse term and add a small roughness-driven highlight.
fn environment_color(texture: &ModelTexture, normal: Vec3) -> [f32; 3] {
    let normal = if normal.length_squared() > 0.0 {
        normal.normalize()
    } else {
        Vec3::Y
    };
    let u = normal.x.atan2(normal.z) / std::f32::consts::TAU + 0.5;
    let v = normal.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    let texel = sample_texture(texture, [u, v]);
    [texel[0], texel[1], texel[2]]
}

fn shade_color(
    normal: Vec3,
    base: [f32; 4],
    metallic: f32,
    roughness: f32,
    occlusion: f32,
    environment: Option<IblAmbient>,
) -> [f32; 4] {
    let normal = if normal.length_squared() > 0.0 {
        normal.normalize()
    } else {
        Vec3::Y
    };
    let view = Vec3::Z;
    let light_dir = Vec3::new(0.3, 0.8, 0.5).normalize();
    let half = (view + light_dir).normalize();
    let n_dot_l = normal.dot(light_dir).max(0.0);
    let n_dot_v = normal.dot(view).max(0.001);
    let n_dot_h = normal.dot(half).max(0.0);
    let v_dot_h = view.dot(half).max(0.0);
    let metallic = metallic.clamp(0.0, 1.0);
    let roughness = roughness.clamp(0.04, 1.0);
    let alpha = roughness * roughness;
    let base_rgb = Vec3::new(base[0], base[1], base[2]);

    // Trowbridge-Reitz (GGX) normal distribution.
    let a2 = alpha * alpha;
    let denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let distribution = a2 / (std::f32::consts::PI * denominator * denominator + 1e-6);
    // Smith-Schlick geometry term.
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let geometry = (n_dot_l / (n_dot_l * (1.0 - k) + k)) * (n_dot_v / (n_dot_v * (1.0 - k) + k));
    // Schlick Fresnel with a common 0.04 dielectric F0.
    let f0 = Vec3::splat(0.04).lerp(base_rgb, metallic);
    let fresnel = f0 + (Vec3::ONE - f0) * (1.0 - v_dot_h).powi(5);
    let specular = distribution * geometry * fresnel / (4.0 * n_dot_v * n_dot_l + 1e-6);

    // Energy-conserving diffuse for the metallic workflow.
    let diffuse_weight = (Vec3::ONE - fresnel) * (1.0 - metallic);
    let direct = (diffuse_weight * base_rgb / std::f32::consts::PI + specular) * n_dot_l * 3.0;

    // Image-based ambient: split-sum diffuse/prefiltered specular with the
    // BRDF LUT, falling back to a hemisphere when no envmap is loaded.
    let hemisphere = match environment {
        Some(ibl) => Vec3::from_array(ibl.diffuse),
        None => Vec3::new(0.22, 0.19, 0.16).lerp(Vec3::new(0.55, 0.65, 0.85), normal.y * 0.5 + 0.5),
    } * occlusion;
    let ambient_diffuse = hemisphere * base_rgb * (1.0 - metallic);
    let ambient_specular = match environment {
        Some(ibl) => Vec3::from_array(ibl.specular) * (f0 * ibl.scale + Vec3::splat(ibl.bias)),
        None => hemisphere * f0 * (1.0 - roughness),
    } * occlusion;
    let color = direct + ambient_diffuse + ambient_specular;
    [
        color.x.clamp(0.0, 1.0),
        color.y.clamp(0.0, 1.0),
        color.z.clamp(0.0, 1.0),
        base[3],
    ]
}

fn sample_track(keys: &[(f32, [f32; 4])], time: f32) -> Option<[f32; 4]> {
    if keys.is_empty() {
        return None;
    }
    if time <= keys[0].0 || keys.len() == 1 {
        return Some(keys[0].1);
    }
    for pair in keys.windows(2) {
        let (start, a) = pair[0];
        let (end, b) = pair[1];
        if time < end {
            let t = if end > start {
                (time - start) / (end - start)
            } else {
                0.0
            };
            return Some([
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
                a[3] + (b[3] - a[3]) * t,
            ]);
        }
    }
    keys.last().map(|(_, value)| *value)
}

pub fn load(path: &std::path::Path) -> anyhow::Result<ModelAsset> {
    let (document, buffers, images) = gltf::import(path)?;
    let mut nodes = Vec::new();
    let mut node_index = std::collections::HashMap::new();
    for node in document.nodes() {
        node_index.insert(node.index(), nodes.len());
        let (translation, rotation, scale) = node.transform().decomposed();
        nodes.push(ModelNode {
            parent: None,
            base_translation: Vec3::from_array(translation),
            base_rotation: Quat::from_array(rotation).normalize(),
            base_scale: Vec3::from_array(scale),
        });
    }
    for node in document.nodes() {
        let Some(&parent) = node_index.get(&node.index()) else {
            continue;
        };
        for child in node.children() {
            if let Some(&index) = node_index.get(&child.index()) {
                nodes[index].parent = Some(parent);
            }
        }
    }

    let mut draws = Vec::new();
    for node in document.nodes() {
        let Some(&node_index_value) = node_index.get(&node.index()) else {
            continue;
        };
        let Some(mesh) = node.mesh() else {
            continue;
        };
        let node_skin = node.skin();
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|coords| coords.into_f32().collect())
                .unwrap_or_default();
            let joints: Vec<[u16; 4]> = reader
                .read_joints(0)
                .map(|values| values.into_u16().collect())
                .unwrap_or_default();
            let weights: Vec<[f32; 4]> = reader
                .read_weights(0)
                .map(|values| values.into_f32().collect())
                .unwrap_or_default();
            let skin = node_skin.as_ref().and_then(|skin| {
                let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));
                let inverse_bind: Vec<Mat4> = reader
                    .read_inverse_bind_matrices()?
                    .map(|matrix| Mat4::from_cols_array_2d(&matrix))
                    .collect();
                let joints = skin
                    .joints()
                    .filter_map(|joint| node_index.get(&joint.index()).copied())
                    .collect();
                Some(ModelSkin {
                    joints,
                    inverse_bind,
                })
            });
            let targets: Vec<Vec<[f32; 3]>> = primitive
                .reader(|buffer| Some(&buffers[buffer.index()]))
                .read_morph_targets()
                .filter_map(|(positions, _, _)| positions.map(|values| values.collect()))
                .collect();
            let morph_weights: Vec<f32> = node
                .weights()
                .or_else(|| mesh.weights())
                .map(|weights| weights.to_vec())
                .unwrap_or_default();
            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|values| values.collect())
                .unwrap_or_default();
            let material = primitive.material();
            let pbr = material.pbr_metallic_roughness();
            let metallic = pbr.metallic_factor();
            let roughness = pbr.roughness_factor();
            let color = pbr.base_color_factor();
            let emissive_factor = material.emissive_factor();
            let normal_map = material
                .normal_texture()
                .and_then(|info| images.get(info.texture().source().index()))
                .and_then(rgba_image);
            let normal_scale = material
                .normal_texture()
                .map(|info| info.scale())
                .unwrap_or(1.0);
            let emissive_map = material
                .emissive_texture()
                .and_then(|info| images.get(info.texture().source().index()))
                .and_then(rgba_image);
            let orm_map = material
                .occlusion_texture()
                .map(|info| info.texture())
                .or_else(|| pbr.metallic_roughness_texture().map(|info| info.texture()))
                .and_then(|texture| images.get(texture.source().index()))
                .and_then(rgba_image);
            let color = [
                color[0].clamp(0.0, 1.0),
                color[1].clamp(0.0, 1.0),
                color[2].clamp(0.0, 1.0),
                color[3].clamp(0.0, 1.0),
            ];
            let texture = material
                .pbr_metallic_roughness()
                .base_color_texture()
                .and_then(|info| images.get(info.texture().source().index()))
                .and_then(rgba_image);
            let mut indices = Vec::new();
            if let Some(source) = reader.read_indices() {
                indices.extend(source.into_u32());
            } else {
                indices.extend(0..positions.len() as u32);
            }
            let vertices: Vec<ModelVertex> = indices
                .into_iter()
                .filter_map(|index| {
                    let index = index as usize;
                    positions.get(index).map(|position| ModelVertex {
                        pos: *position,
                        uv: uvs.get(index).copied().unwrap_or([0.0, 0.0]),
                        color,
                        joints: joints.get(index).copied().unwrap_or([0; 4]),
                        weights: weights.get(index).copied().unwrap_or([0.0; 4]),
                        normal: normals.get(index).copied().unwrap_or([0.0, 1.0, 0.0]),
                    })
                })
                .collect();
            if !vertices.is_empty() {
                draws.push(ModelDraw {
                    vertices,
                    node: node_index_value,
                    texture,
                    skin: skin.clone(),
                    targets,
                    weights: morph_weights.clone(),
                    metallic,
                    roughness,
                    normal_map,
                    normal_scale,
                    emissive_map,
                    emissive_factor,
                    orm_map,
                });
            }
        }
    }
    if draws.is_empty() {
        anyhow::bail!("glTF has no triangle geometry");
    }

    let animations: Vec<ModelAnimation> = document
        .animations()
        .filter_map(|animation| {
            let mut tracks = Vec::new();
            let mut weight_tracks = Vec::new();
            for channel in animation.channels() {
                let target = channel.target();
                let Some(&node) = node_index.get(&target.node().index()) else {
                    continue;
                };
                let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
                let Some(inputs) = reader.read_inputs() else {
                    continue;
                };
                let times: Vec<f32> = inputs.collect();
                if target.property() == gltf::animation::Property::MorphTargetWeights {
                    if let Some(gltf::animation::util::ReadOutputs::MorphTargetWeights(values)) =
                        reader.read_outputs()
                    {
                        let values: Vec<f32> = match values {
                            gltf::animation::util::MorphTargetWeights::I8(values) => {
                                values.map(|value| value as f32).collect()
                            }
                            gltf::animation::util::MorphTargetWeights::U8(values) => {
                                values.map(|value| value as f32).collect()
                            }
                            gltf::animation::util::MorphTargetWeights::I16(values) => {
                                values.map(|value| value as f32).collect()
                            }
                            gltf::animation::util::MorphTargetWeights::U16(values) => {
                                values.map(|value| value as f32).collect()
                            }
                            gltf::animation::util::MorphTargetWeights::F32(values) => {
                                values.collect()
                            }
                        };
                        let target_count = target
                            .node()
                            .mesh()
                            .and_then(|mesh| mesh.primitives().next())
                            .map(|primitive| primitive.morph_targets().len())
                            .unwrap_or(1)
                            .max(1);
                        let keys = times
                            .into_iter()
                            .zip(values.chunks(target_count))
                            .map(|(time, values)| (time, values.to_vec()))
                            .collect();
                        weight_tracks.push(WeightTrack { node, keys });
                    }
                    continue;
                }
                let property = match target.property() {
                    gltf::animation::Property::Translation => Property::Translation,
                    gltf::animation::Property::Rotation => Property::Rotation,
                    gltf::animation::Property::Scale => Property::Scale,
                    gltf::animation::Property::MorphTargetWeights => continue,
                };
                let Some(outputs) = reader.read_outputs() else {
                    continue;
                };
                let values: Vec<[f32; 4]> = match outputs {
                    gltf::animation::util::ReadOutputs::Translations(values) => values
                        .map(|value| [value[0], value[1], value[2], 1.0])
                        .collect(),
                    gltf::animation::util::ReadOutputs::Rotations(values) => {
                        values.into_f32().collect()
                    }
                    gltf::animation::util::ReadOutputs::Scales(values) => values
                        .map(|value| [value[0], value[1], value[2], 1.0])
                        .collect(),
                    _ => continue,
                };
                let keys = times.into_iter().zip(values).collect();
                tracks.push(Track {
                    node,
                    property,
                    keys,
                });
            }
            if tracks.is_empty() && weight_tracks.is_empty() {
                None
            } else {
                Some(ModelAnimation {
                    name: animation.name().map(str::to_string),
                    tracks,
                    weight_tracks,
                })
            }
        })
        .collect();

    let mut asset = ModelAsset {
        draws,
        nodes,
        animations,
        clip: 0,
        environment: None,
        world0: Vec::new(),
        placement: None,
    };
    asset.world0 = asset.world_transforms(0.0);
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for draw in &asset.draws {
        let world = asset
            .world0
            .get(draw.node)
            .copied()
            .unwrap_or(Mat4::IDENTITY);
        for vertex in &draw.vertices {
            let point = world.transform_point3(Vec3::from_array(vertex.pos));
            min = min.min(point);
            max = max.max(point);
        }
    }
    let center = (min + max) * 0.5;
    let extent = (max - min).max_element().max(0.0001);
    asset.placement = Some(Placement {
        center,
        min_y: min.y,
        scale: (GAME_HEIGHT as f32 * 0.7) / extent,
        ground: GAME_HEIGHT as f32 * 0.88,
    });
    Ok(asset)
}

fn rgba_image(image: &gltf::image::Data) -> Option<ModelTexture> {
    use gltf::image::Format;
    let mut rgba = Vec::with_capacity((image.width * image.height * 4) as usize);
    match image.format {
        Format::R8 => {
            for value in &image.pixels {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
        }
        Format::R8G8 => {
            for pixel in image.pixels.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        Format::R8G8B8 => {
            for pixel in image.pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        Format::R8G8B8A8 => rgba.extend_from_slice(&image.pixels),
        _ => return None,
    }
    Some(ModelTexture {
        width: image.width,
        height: image.height,
        rgba,
    })
}

pub fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("manato model material layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

pub fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("manato model shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};
@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(
        in.pos.x / 320.0 * 2.0 - 1.0,
        1.0 - in.pos.y / 240.0 * 2.0,
        in.pos.z,
        1.0,
    );
    out.uv = in.uv;
    out.color = in.color;
    return out;
}
@group(0) @binding(0) var base_color: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(base_color, samp, in.uv) * in.color;
}
"#
            .into(),
        ),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("manato model pipeline layout"),
        bind_group_layouts: &[layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("manato model pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ModelVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    wgpu::VertexAttribute {
                        offset: 12,
                        shader_location: 1,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                    wgpu::VertexAttribute {
                        offset: 20,
                        shader_location: 2,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                ],
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

pub fn upload_model(device: &wgpu::Device, vertices: &[ModelVertex]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("manato model vertices"),
        contents: bytemuck::cast_slice(vertices),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefiltered_levels_and_brdf_lut_are_roughness_aware() {
        let texture = ModelTexture {
            width: 32,
            height: 16,
            rgba: (0..(32 * 16))
                .flat_map(|index| {
                    let value = (index % 256) as u8;
                    [value, 255 - value, 128, 255]
                })
                .collect(),
        };
        let environment = PrefilteredEnvironment::new(texture);
        assert_eq!(environment.levels.len(), 5);
        assert!(environment.levels[4].width < environment.levels[0].width);
        assert_ne!(
            environment.sample(Vec3::Z, 0.0),
            environment.sample(Vec3::Z, 1.0)
        );
        let (glossy_scale, glossy_bias) = brdf_lut(0.7, 0.1);
        let (rough_scale, rough_bias) = brdf_lut(0.7, 0.9);
        assert!((0.0..=1.0).contains(&glossy_scale));
        assert!((0.0..=1.0).contains(&glossy_bias));
        assert!((0.0..=1.0).contains(&rough_scale));
        assert!((0.0..=1.0).contains(&rough_bias));
        assert!((glossy_bias - rough_bias).abs() > 1e-4);
    }

    #[test]
    fn samples_equirectangular_environment() {
        let texture = ModelTexture {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let front = environment_color(&texture, Vec3::Z);
        let back = environment_color(&texture, Vec3::NEG_Z);
        assert_ne!(front, back);
        assert!(front[2] > front[0], "front samples the blue half");
    }

    #[test]
    fn directional_lighting_uses_normals_and_pbr_factors() {
        let lit = shade_color(
            Vec3::new(0.0, 0.8, 0.5).normalize(),
            [1.0; 4],
            0.0,
            1.0,
            1.0,
            None,
        );
        assert!(lit[0] <= 1.0 && lit[0] > 0.4);
        let unlit = shade_color(Vec3::new(0.0, -1.0, 0.0), [1.0; 4], 0.0, 1.0, 1.0, None);
        assert!(unlit[0] > 0.0 && unlit[0] < 0.35);
        let occluded = shade_color(Vec3::new(0.0, -1.0, 0.0), [1.0; 4], 0.0, 1.0, 0.5, None);
        assert!((occluded[0] - unlit[0] * 0.5).abs() < 1e-5);
        let metallic = shade_color(
            Vec3::new(0.0, 0.8, 0.5).normalize(),
            [1.0; 4],
            1.0,
            1.0,
            1.0,
            None,
        );
        assert!(metallic[0] < lit[0]);
        let glossy = shade_color(
            Vec3::new(0.0, 0.8, 0.5).normalize(),
            [1.0; 4],
            0.0,
            0.0,
            1.0,
            None,
        );
        assert!(glossy[0] >= lit[0]);
    }

    #[test]
    fn node_overrides_apply_scale_and_depth() {
        let dir = std::env::temp_dir().join(format!("manato-model-ctrl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("model.gltf");
        std::fs::write(
            &path,
            r#"{
  "asset": {"version": "2.0"},
  "buffers": [{"uri": "data:application/octet-stream;base64,AACAvwAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAEAAAAAA", "byteLength": 36}],
  "bufferViews": [{"buffer": 0, "byteLength": 36, "target": 34962}],
  "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1,0,0], "max": [1,2,0]}],
  "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1,1,1,1]}}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "material": 0, "mode": 4}]}],
  "nodes": [{"mesh": 0}],
  "scenes": [{"nodes": [0]}],
  "scene": 0
}"#,
        )
        .expect("model");
        let asset = load(&path).expect("load");
        let base = asset.sample(0.0, &Default::default());
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(0, (10.0, 2.0, 2.0));
        let scaled = asset.sample(0.0, &overrides);
        assert!((scaled[0][0].pos[0] - base[0][0].pos[0]).abs() > 1.0);
        assert!((scaled[0][0].pos[2] - 0.97).abs() < 1e-5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_base_color_texture_and_uvs() {
        let dir = std::env::temp_dir().join(format!("manato-model-tex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("textured.gltf");
        std::fs::write(
            &path,
            r#"{
  "asset": {"version": "2.0"},
  "buffers": [{"uri": "data:application/octet-stream;base64,AACAvwAAgL8AAAAAAACAPwAAgL8AAAAAAAAAAAAAgD8AAAAAAAAAAAAAgD8AAIA/AACAPwAAAD8AAAAA", "byteLength": 48}],
  "bufferViews": [
    {"buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962},
    {"buffer": 0, "byteOffset": 36, "byteLength": 24, "target": 34962}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1,-1,0], "max": [1,1,0]},
    {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"}
  ],
  "images": [
    {"uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="},
    {"uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="},
    {"uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="},
    {"uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}
  ],
  "samplers": [{}],
  "textures": [{"source": 0, "sampler": 0}, {"source": 1, "sampler": 0}, {"source": 2, "sampler": 0}, {"source": 3, "sampler": 0}],
  "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "baseColorFactor": [1,1,1,1]}, "normalTexture": {"index": 1}, "emissiveTexture": {"index": 2}, "emissiveFactor": [1,0,0], "occlusionTexture": {"index": 3}}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "material": 0, "mode": 4}]}],
  "nodes": [{"mesh": 0}],
  "scenes": [{"nodes": [0]}],
  "scene": 0
}"#,
        )
        .expect("model");
        let asset = load(&path).expect("load");
        assert_eq!(asset.draws.len(), 1);
        let texture = asset.draws[0].texture.as_ref().expect("texture");
        assert_eq!((texture.width, texture.height), (1, 1));
        assert_eq!(&texture.rgba[..4], &[255, 0, 0, 255]);
        let draw = &asset.draws[0];
        assert!(draw.normal_map.is_some());
        assert!(draw.emissive_map.is_some());
        assert_eq!(draw.emissive_factor, [1.0, 0.0, 0.0]);
        assert!(draw.orm_map.is_some());
        assert_eq!(sample_texture(texture, [0.5, 0.5]), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(asset.sample(0.0, &Default::default())[0].len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn morph_target_weight_animation_moves_vertices() {
        let dir = std::env::temp_dir().join(format!("manato-model-morph-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("morph.gltf");
        std::fs::write(
            &path,
            r#"{
  "asset": {"version": "2.0"},
  "buffers": [
    {"uri": "data:application/octet-stream;base64,AACAvwAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAEAAAAAA", "byteLength": 36},
    {"uri": "data:application/octet-stream;base64,AAAAQAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAQAAAAAAAAAAA", "byteLength": 36},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAgD8=", "byteLength": 8},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAgD8=", "byteLength": 8}
  ],
  "bufferViews": [
    {"buffer": 0, "byteLength": 36, "target": 34962},
    {"buffer": 1, "byteLength": 36, "target": 34962},
    {"buffer": 2, "byteLength": 8},
    {"buffer": 3, "byteLength": 8}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1,0,0], "max": [1,2,0]},
    {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
    {"bufferView": 2, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
    {"bufferView": 3, "componentType": 5126, "count": 2, "type": "SCALAR"}
  ],
  "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1,1,1,1]}}],
  "meshes": [{"weights": [0.0], "primitives": [{"attributes": {"POSITION": 0}, "targets": [{"POSITION": 1}], "material": 0, "mode": 4}]}],
  "nodes": [{"mesh": 0, "weights": [0.0]}],
  "scenes": [{"nodes": [0]}],
  "scene": 0,
  "animations": [{"channels": [{"sampler": 0, "target": {"node": 0, "path": "weights"}}],
                  "samplers": [{"input": 2, "output": 3, "interpolation": "LINEAR"}]}]
}"#,
        )
        .expect("model");
        let asset = load(&path).expect("load");
        assert_eq!(asset.draws.len(), 1);
        assert_eq!(asset.draws[0].targets.len(), 1);
        let start = asset.sample(0.0, &Default::default())[0][0].pos;
        let end = asset.sample(1.0, &Default::default())[0][0].pos;
        assert!(
            (end[0] - start[0]).abs() > 1.0,
            "morph weight animated the target"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skinned_joint_animation_moves_vertices() {
        let dir = std::env::temp_dir().join(format!("manato-model-skin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("skinned.gltf");
        std::fs::write(
            &path,
            r#"{
  "asset": {"version": "2.0"},
  "buffers": [
    {"uri": "data:application/octet-stream;base64,AACAvwAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAEAAAAAA", "byteLength": 36},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "byteLength": 24},
    {"uri": "data:application/octet-stream;base64,AACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAA", "byteLength": 48},
    {"uri": "data:application/octet-stream;base64,AACAPwAAAAAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAAAAAACAPw==", "byteLength": 64},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAgD8=", "byteLength": 8},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAA", "byteLength": 24}
  ],
  "bufferViews": [
    {"buffer": 0, "byteLength": 36, "target": 34962},
    {"buffer": 1, "byteLength": 24, "target": 34962},
    {"buffer": 2, "byteLength": 48, "target": 34962},
    {"buffer": 3, "byteLength": 64},
    {"buffer": 4, "byteLength": 8},
    {"buffer": 5, "byteLength": 24}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1,0,0], "max": [1,2,0]},
    {"bufferView": 1, "componentType": 5123, "count": 3, "type": "VEC4"},
    {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"},
    {"bufferView": 3, "componentType": 5126, "count": 1, "type": "MAT4"},
    {"bufferView": 4, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
    {"bufferView": 5, "componentType": 5126, "count": 2, "type": "VEC3"}
  ],
  "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1,1,1,1]}}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}, "material": 0, "mode": 4}]}],
  "nodes": [{"mesh": 0, "skin": 0}, {"name": "joint"}],
  "skins": [{"joints": [1], "inverseBindMatrices": 3}],
  "scenes": [{"nodes": [0, 1]}],
  "scene": 0,
  "animations": [{"channels": [{"sampler": 0, "target": {"node": 1, "path": "translation"}}],
                  "samplers": [{"input": 4, "output": 5, "interpolation": "LINEAR"}]}]
}"#,
        )
        .expect("model");
        let asset = load(&path).expect("load");
        assert_eq!(asset.draws.len(), 1);
        assert!(asset.draws[0].skin.is_some());
        let start = asset.sample(0.0, &Default::default())[0][0].pos;
        let end = asset.sample(1.0, &Default::default())[0][0].pos;
        assert!(
            (end[0] - start[0]).abs() > 1.0,
            "joint animation deformed the mesh"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selects_between_multiple_animation_clips() {
        let dir = std::env::temp_dir().join(format!("manato-model-clips-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("clips.gltf");
        std::fs::write(
            &path,
            r#"{
  "asset": {"version": "2.0"},
  "buffers": [
    {"uri": "data:application/octet-stream;base64,AACAvwAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAEAAAAAA", "byteLength": 36},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAgD8=", "byteLength": 8},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAA", "byteLength": 24},
    {"uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAvwAAAAAAAAAA", "byteLength": 24}
  ],
  "bufferViews": [
    {"buffer": 0, "byteLength": 36, "target": 34962},
    {"buffer": 1, "byteLength": 8},
    {"buffer": 2, "byteLength": 24},
    {"buffer": 3, "byteLength": 24}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1,0,0], "max": [1,2,0]},
    {"bufferView": 1, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
    {"bufferView": 2, "componentType": 5126, "count": 2, "type": "VEC3"},
    {"bufferView": 3, "componentType": 5126, "count": 2, "type": "VEC3"}
  ],
  "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1,1,1,1]}}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "material": 0, "mode": 4}]}],
  "nodes": [{"mesh": 0}],
  "scenes": [{"nodes": [0]}],
  "scene": 0,
  "animations": [
    {"name": "slide", "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}], "samplers": [{"input": 1, "output": 2, "interpolation": "LINEAR"}]},
    {"name": "back", "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}], "samplers": [{"input": 1, "output": 3, "interpolation": "LINEAR"}]}
  ]
}"#,
        )
        .expect("model");
        let mut asset = load(&path).expect("load");
        assert_eq!(asset.clip_names(), vec!["slide", "back"]);
        let slide = asset.sample(1.0, &Default::default())[0][0].pos[0];
        assert!(asset.select_clip("back"));
        let back = asset.sample(1.0, &Default::default())[0][0].pos[0];
        assert!(
            (slide - back).abs() > 1.0,
            "clips move the node differently"
        );
        assert!(asset.select_clip("1"));
        assert_eq!(asset.sample(1.0, &Default::default())[0][0].pos[0], back);
        assert!(!asset.select_clip("missing"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn node_translation_animation_moves_vertices() {
        let dir = std::env::temp_dir().join(format!("manato-model-anim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("animated.gltf");
        std::fs::write(
            &path,
            r#"{
  "asset": {"version": "2.0"},
  "buffers": [{"uri": "data:application/octet-stream;base64,AACAvwAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAEAAAAAA", "byteLength": 36},
              {"uri": "data:application/octet-stream;base64,AAAAAAAAgD8AAAAA", "byteLength": 8},
              {"uri": "data:application/octet-stream;base64,AAAAAAAAgD8AAAAAAACAPwAAAAAAAAAA", "byteLength": 24}],
  "bufferViews": [
    {"buffer": 0, "byteLength": 36, "target": 34962},
    {"buffer": 1, "byteLength": 8},
    {"buffer": 2, "byteLength": 24}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1,0,0], "max": [1,2,0]},
    {"bufferView": 1, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
    {"bufferView": 2, "componentType": 5126, "count": 2, "type": "VEC3"}
  ],
  "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1,1,1,1]}}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "material": 0, "mode": 4}]}],
  "nodes": [{"mesh": 0}],
  "scenes": [{"nodes": [0]}],
  "scene": 0,
  "animations": [{"channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}],
                  "samplers": [{"input": 1, "output": 2, "interpolation": "LINEAR"}]}]
}"#,
        )
        .expect("model");
        let asset = load(&path).expect("load");
        let start = asset.sample(0.0, &Default::default())[0][0].pos;
        let middle = asset.sample(1.0, &Default::default())[0][0].pos;
        assert!(
            (middle[0] - start[0]).abs() > 1.0,
            "animation translated the node"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
