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
    /// Split-sum LUT coordinates `(NdotV, roughness)`.
    pub brdf_uv: [f32; 2],
    /// Prefiltered environment specular radiance.
    pub specular: [f32; 3],
    /// Fresnel F0.
    pub f0: [f32; 3],
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
    clearcoat: f32,
    clearcoat_roughness: f32,
    transmission: f32,
    anisotropy: f32,
    anisotropy_rotation: f32,
    sheen_color: [f32; 3],
    sheen_roughness: f32,
    iridescence: f32,
    iridescence_roughness: f32,
    iridescence_ior: f32,
    iridescence_thickness: [f32; 2],
    thickness: f32,
    attenuation_color: [f32; 3],
    attenuation_distance: f32,
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

#[derive(Debug, Clone)]
pub struct ModelAsset {
    pub draws: Vec<ModelDraw>,
    nodes: Vec<ModelNode>,
    animations: Vec<ModelAnimation>,
    clip: usize,
    environment: Option<PrefilteredEnvironment>,
    lights: Vec<Light>,
    world0: Vec<Mat4>,
    placement: Option<Placement>,
}

impl Default for ModelAsset {
    fn default() -> Self {
        Self {
            draws: Vec::new(),
            nodes: Vec::new(),
            animations: Vec::new(),
            clip: 0,
            environment: None,
            lights: default_lights().to_vec(),
            world0: Vec::new(),
            placement: None,
        }
    }
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

    /// Replaces the default two-light rig with content-defined lights.
    pub fn set_lights(&mut self, lights: &[manato_formats::stage::ModelLightDef]) {
        if lights.is_empty() {
            return;
        }
        self.lights = lights
            .iter()
            .map(|light| Light {
                direction: Vec3::from_array(light.direction).normalize_or_zero(),
                color: Vec3::from_array(light.color),
                intensity: light.intensity.max(0.0),
            })
            .filter(|light| light.direction.length_squared() > 0.0)
            .collect();
        if self.lights.is_empty() {
            self.lights = default_lights().to_vec();
        }
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
                            IblAmbient {
                                diffuse: environment_color(environment.base(), normal),
                                specular: environment.sample(reflect, roughness),
                            }
                        });
                        let sample = MaterialSample {
                            metallic,
                            roughness,
                            occlusion,
                            clearcoat: draw.clearcoat,
                            clearcoat_roughness: draw.clearcoat_roughness,
                            transmission: draw.transmission,
                            anisotropy: draw.anisotropy,
                            anisotropy_rotation: draw.anisotropy_rotation,
                            sheen_color: draw.sheen_color,
                            sheen_roughness: draw.sheen_roughness,
                            iridescence: draw.iridescence,
                            iridescence_roughness: draw.iridescence_roughness,
                            iridescence_ior: draw.iridescence_ior,
                            iridescence_thickness: draw.iridescence_thickness,
                            thickness: draw.thickness,
                            attenuation_color: draw.attenuation_color,
                            attenuation_distance: draw.attenuation_distance,
                        };
                        let shaded = shade_color_parts(
                            normal,
                            out.color,
                            environment,
                            &self.lights,
                            &sample,
                        );
                        out.color = [shaded.color.x, shaded.color.y, shaded.color.z, out.color[3]];
                        out.brdf_uv = shaded.brdf_uv;
                        out.specular = shaded.specular.to_array();
                        out.f0 = shaded.f0.to_array();
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

#[derive(Debug, Clone, Copy)]
struct Light {
    direction: Vec3,
    color: Vec3,
    intensity: f32,
}

/// Two-light studio rig: a bright key and a soft cool fill.
fn default_lights() -> [Light; 2] {
    [
        Light {
            direction: Vec3::new(0.3, 0.8, 0.5).normalize(),
            color: Vec3::ONE,
            intensity: 1.0,
        },
        Light {
            direction: Vec3::new(-0.45, 0.25, -0.6).normalize(),
            color: Vec3::new(0.55, 0.65, 0.9),
            intensity: 0.35,
        },
    ]
}

#[cfg(test)]
fn shade_color(
    normal: Vec3,
    base: [f32; 4],
    metallic: f32,
    roughness: f32,
    occlusion: f32,
    environment: Option<IblAmbient>,
) -> [f32; 4] {
    shade_color_with(
        normal,
        base,
        metallic,
        roughness,
        occlusion,
        environment,
        &default_lights(),
    )
}

struct Shaded {
    color: Vec3,
    specular: Vec3,
    f0: Vec3,
    brdf_uv: [f32; 2],
}

/// Surface material parameters sampled from a glTF material and its maps.
#[derive(Debug, Clone, Copy)]
struct MaterialSample {
    metallic: f32,
    roughness: f32,
    occlusion: f32,
    clearcoat: f32,
    clearcoat_roughness: f32,
    transmission: f32,
    anisotropy: f32,
    anisotropy_rotation: f32,
    sheen_color: [f32; 3],
    sheen_roughness: f32,
    iridescence: f32,
    iridescence_roughness: f32,
    iridescence_ior: f32,
    iridescence_thickness: [f32; 2],
    thickness: f32,
    attenuation_color: [f32; 3],
    attenuation_distance: f32,
}

impl Default for MaterialSample {
    fn default() -> Self {
        Self {
            metallic: 0.0,
            roughness: 0.5,
            occlusion: 1.0,
            clearcoat: 0.0,
            clearcoat_roughness: 0.0,
            transmission: 0.0,
            anisotropy: 0.0,
            anisotropy_rotation: 0.0,
            sheen_color: [0.0; 3],
            sheen_roughness: 0.0,
            iridescence: 0.0,
            iridescence_roughness: 0.0,
            iridescence_ior: 1.3,
            iridescence_thickness: [100.0, 400.0],
            thickness: 0.0,
            attenuation_color: [1.0; 3],
            attenuation_distance: f32::INFINITY,
        }
    }
}

#[cfg(test)]
fn shade_color_with(
    normal: Vec3,
    base: [f32; 4],
    metallic: f32,
    roughness: f32,
    occlusion: f32,
    environment: Option<IblAmbient>,
    lights: &[Light],
) -> [f32; 4] {
    let sample = MaterialSample {
        metallic,
        roughness,
        occlusion,
        ..Default::default()
    };
    let shaded = shade_color_parts(normal, base, environment, lights, &sample);
    let mut color = shaded.color;
    if let Some(_ibl) = environment {
        let (scale, bias) = brdf_lut(shaded.brdf_uv[0], shaded.brdf_uv[1]);
        color += shaded.specular * (shaded.f0 * scale + Vec3::splat(bias));
    }
    [
        color.x.clamp(0.0, 1.0),
        color.y.clamp(0.0, 1.0),
        color.z.clamp(0.0, 1.0),
        base[3],
    ]
}

/// Approximates thin-film interference as a per-channel phase shift over the
/// visible spectrum, returning a multiplicative tint in `[0, 1]`.
fn iridescence_tint(n_dot_v: f32, ior: f32, thickness: [f32; 2]) -> Vec3 {
    let thickness = (thickness[0] + thickness[1]).max(0.0) * 0.5;
    if thickness <= 0.0 {
        return Vec3::ONE;
    }
    let cos_theta = n_dot_v.clamp(0.0, 1.0);
    let mut channels = [1.0f32; 3];
    for (channel, wavelength) in channels.iter_mut().zip([680.0f32, 550.0, 440.0]) {
        let phase = 4.0 * std::f32::consts::PI * ior.max(1.0) * thickness * cos_theta / wavelength;
        *channel = (0.5 + 0.5 * phase.cos()).clamp(0.0, 1.0);
    }
    Vec3::from_array(channels)
}

/// Beer-Lambert transmittance through a volume of the given thickness.
fn volume_transmittance(thickness: f32, attenuation_color: [f32; 3], distance: f32) -> Vec3 {
    if thickness <= 0.0 || !distance.is_finite() || distance <= 0.0 {
        return Vec3::ONE;
    }
    let color = Vec3::from_array(attenuation_color).clamp(Vec3::splat(1e-4), Vec3::ONE);
    Vec3::new(
        (color.x.ln() / distance * thickness).exp(),
        (color.y.ln() / distance * thickness).exp(),
        (color.z.ln() / distance * thickness).exp(),
    )
}

fn shade_color_parts(
    normal: Vec3,
    base: [f32; 4],
    environment: Option<IblAmbient>,
    lights: &[Light],
    sample: &MaterialSample,
) -> Shaded {
    let normal = if normal.length_squared() > 0.0 {
        normal.normalize()
    } else {
        Vec3::Y
    };
    let view = Vec3::Z;
    let n_dot_v = normal.dot(view).max(0.001);
    let occlusion = sample.occlusion.clamp(0.0, 1.0);
    let metallic = sample.metallic.clamp(0.0, 1.0);
    let roughness = sample.roughness.clamp(0.04, 1.0);
    let alpha = roughness * roughness;
    let base_rgb = Vec3::new(base[0], base[1], base[2]);
    let f0 = Vec3::splat(0.04).lerp(base_rgb, metallic);
    let clearcoat = sample.clearcoat.clamp(0.0, 1.0);
    let clearcoat_roughness = sample.clearcoat_roughness.clamp(0.04, 1.0);
    let transmission = sample.transmission.clamp(0.0, 1.0);
    let clearcoat_f0 = Vec3::splat(0.04);
    let anisotropy = sample.anisotropy.clamp(0.0, 1.0);
    let up = if normal.y.abs() < 0.999 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let tangent = up.cross(normal).normalize_or_zero();
    let bitangent = normal.cross(tangent);
    let rotation = sample.anisotropy_rotation;
    let aniso_tangent = tangent * rotation.cos() + bitangent * rotation.sin();
    let aniso_bitangent = -tangent * rotation.sin() + bitangent * rotation.cos();
    let aniso_alpha_t = (roughness * (1.0 + anisotropy * 0.9)).clamp(0.04, 1.0);
    let aniso_alpha_b = (roughness * (1.0 - anisotropy * 0.7)).clamp(0.04, 1.0);
    let sheen_color = Vec3::from_array(sample.sheen_color).clamp(Vec3::ZERO, Vec3::ONE);
    let sheen_roughness = sample.sheen_roughness.clamp(0.04, 1.0);
    let iridescence = sample.iridescence.clamp(0.0, 1.0);
    let iridescence_tint = if iridescence > 0.0 {
        let tint = iridescence_tint(
            n_dot_v,
            sample.iridescence_ior,
            sample.iridescence_thickness,
        );
        Vec3::ONE.lerp(tint, 1.0 - sample.iridescence_roughness.clamp(0.0, 1.0))
    } else {
        Vec3::ONE
    };
    let specular_tint = Vec3::ONE.lerp(iridescence_tint, iridescence);

    let mut direct = Vec3::ZERO;
    for light in lights {
        let half = (view + light.direction).normalize();
        let n_dot_l = normal.dot(light.direction).max(0.0);
        if n_dot_l <= 0.0 {
            continue;
        }
        let n_dot_h = normal.dot(half).max(0.0);
        let v_dot_h = view.dot(half).max(0.0);
        // Trowbridge-Reitz (GGX) normal distribution.
        let a2 = alpha * alpha;
        let denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
        let distribution = if anisotropy > 0.0 {
            let h_dot_t = half.dot(aniso_tangent);
            let h_dot_b = half.dot(aniso_bitangent);
            let at2 = aniso_alpha_t * aniso_alpha_t;
            let ab2 = aniso_alpha_b * aniso_alpha_b;
            let denom = h_dot_t * h_dot_t / (at2 * at2)
                + h_dot_b * h_dot_b / (ab2 * ab2)
                + n_dot_h * n_dot_h;
            at2 * ab2 / (std::f32::consts::PI * denom * denom + 1e-6)
        } else {
            a2 / (std::f32::consts::PI * denominator * denominator + 1e-6)
        };
        // Smith-Schlick geometry term.
        let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
        let geometry =
            (n_dot_l / (n_dot_l * (1.0 - k) + k)) * (n_dot_v / (n_dot_v * (1.0 - k) + k));
        let fresnel = f0 + (Vec3::ONE - f0) * (1.0 - v_dot_h).powi(5);
        let specular =
            distribution * geometry * fresnel / (4.0 * n_dot_v * n_dot_l + 1e-6) * specular_tint;
        let diffuse_weight = (Vec3::ONE - fresnel)
            * (1.0 - metallic)
            * (1.0 - clearcoat * 0.5)
            * (1.0 - transmission);
        let mut layer = (diffuse_weight * base_rgb / std::f32::consts::PI + specular) * n_dot_l;
        if clearcoat > 0.0 {
            let cc_a = clearcoat_roughness * clearcoat_roughness;
            let cc_a2 = cc_a * cc_a;
            let cc_denominator = n_dot_h * n_dot_h * (cc_a2 - 1.0) + 1.0;
            let cc_distribution =
                cc_a2 / (std::f32::consts::PI * cc_denominator * cc_denominator + 1e-6);
            let cc_k = (clearcoat_roughness + 1.0) * (clearcoat_roughness + 1.0) / 8.0;
            let cc_geometry = (n_dot_l / (n_dot_l * (1.0 - cc_k) + cc_k))
                * (n_dot_v / (n_dot_v * (1.0 - cc_k) + cc_k));
            let cc_fresnel = clearcoat_f0 + (Vec3::ONE - clearcoat_f0) * (1.0 - v_dot_h).powi(5);
            let cc_specular =
                cc_distribution * cc_geometry * cc_fresnel / (4.0 * n_dot_v * n_dot_l + 1e-6);
            layer += cc_specular * n_dot_l * clearcoat;
        }
        if sheen_color.length_squared() > 0.0 {
            let sheen_a = sheen_roughness * sheen_roughness;
            let sheen_denom = n_dot_h * n_dot_h * (sheen_a * sheen_a - 1.0) + 1.0;
            let sheen_distribution =
                sheen_a * sheen_a / (std::f32::consts::PI * sheen_denom * sheen_denom + 1e-6);
            let sheen_fresnel = (1.0 - v_dot_h).clamp(0.0, 1.0).powi(5);
            layer += sheen_color * sheen_distribution * n_dot_l * sheen_fresnel * 0.5;
        }
        direct += layer * light.color * light.intensity;
    }
    let direct = direct * 3.0;

    // Image-based ambient: split-sum diffuse/prefiltered specular with the
    // BRDF LUT, falling back to a hemisphere when no envmap is loaded.
    let hemisphere = match environment {
        Some(ibl) => Vec3::from_array(ibl.diffuse),
        None => Vec3::new(0.22, 0.19, 0.16).lerp(Vec3::new(0.55, 0.65, 0.85), normal.y * 0.5 + 0.5),
    } * occlusion;
    let ambient_diffuse = hemisphere * base_rgb * (1.0 - metallic) * (1.0 - transmission);
    let transmittance = volume_transmittance(
        sample.thickness,
        sample.attenuation_color,
        sample.attenuation_distance,
    );
    let transmission_tint = hemisphere * base_rgb * transmission * transmittance;
    let clearcoat_ambient =
        hemisphere * clearcoat_f0 * (1.0 - clearcoat_roughness) * clearcoat * occlusion;
    let sheen_ambient =
        sheen_color * (1.0 - n_dot_v).clamp(0.0, 1.0).powi(5) * (1.0 - sheen_roughness) * occlusion;
    match environment {
        Some(ibl) => Shaded {
            color: direct + ambient_diffuse + transmission_tint + clearcoat_ambient + sheen_ambient,
            specular: Vec3::from_array(ibl.specular) * occlusion * specular_tint,
            f0,
            brdf_uv: [n_dot_v, roughness],
        },
        None => Shaded {
            color: direct
                + ambient_diffuse
                + transmission_tint
                + clearcoat_ambient
                + sheen_ambient
                + hemisphere * f0 * (1.0 - roughness) * occlusion,
            specular: Vec3::ZERO,
            f0,
            brdf_uv: [0.0, 0.0],
        },
    }
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
            let clearcoat = material
                .extension_value("KHR_materials_clearcoat")
                .and_then(|value| value.get("clearcoatFactor"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let clearcoat_roughness = material
                .extension_value("KHR_materials_clearcoat")
                .and_then(|value| value.get("clearcoatRoughnessFactor"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let transmission = material
                .extension_value("KHR_materials_transmission")
                .and_then(|value| value.get("transmissionFactor"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let anisotropy = material
                .extension_value("KHR_materials_anisotropy")
                .and_then(|value| value.get("anisotropyStrength"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let anisotropy_rotation = material
                .extension_value("KHR_materials_anisotropy")
                .and_then(|value| value.get("anisotropyRotation"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let sheen_color = material
                .extension_value("KHR_materials_sheen")
                .and_then(|value| value.get("sheenColorFactor"))
                .and_then(|value| value.as_array())
                .filter(|values| values.len() >= 3)
                .map(|values| {
                    [
                        values[0].as_f64().unwrap_or(0.0) as f32,
                        values[1].as_f64().unwrap_or(0.0) as f32,
                        values[2].as_f64().unwrap_or(0.0) as f32,
                    ]
                })
                .unwrap_or([0.0; 3]);
            let sheen_roughness = material
                .extension_value("KHR_materials_sheen")
                .and_then(|value| value.get("sheenRoughnessFactor"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let iridescence = material
                .extension_value("KHR_materials_iridescence")
                .and_then(|value| value.get("iridescenceFactor"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let iridescence_roughness = material
                .extension_value("KHR_materials_iridescence")
                .and_then(|value| value.get("iridescenceRoughness"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let iridescence_ior = material
                .extension_value("KHR_materials_iridescence")
                .and_then(|value| value.get("iridescenceIor"))
                .and_then(|value| value.as_f64())
                .unwrap_or(1.3) as f32;
            let thickness_min = material
                .extension_value("KHR_materials_iridescence")
                .and_then(|value| value.get("iridescenceThicknessMinimum"))
                .and_then(|value| value.as_f64())
                .unwrap_or(100.0) as f32;
            let thickness_max = material
                .extension_value("KHR_materials_iridescence")
                .and_then(|value| value.get("iridescenceThicknessMaximum"))
                .and_then(|value| value.as_f64())
                .unwrap_or(400.0) as f32;
            let thickness = material
                .extension_value("KHR_materials_volume")
                .and_then(|value| value.get("thicknessFactor"))
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0) as f32;
            let attenuation_color = material
                .extension_value("KHR_materials_volume")
                .and_then(|value| value.get("attenuationColor"))
                .and_then(|value| value.as_array())
                .filter(|values| values.len() >= 3)
                .map(|values| {
                    [
                        values[0].as_f64().unwrap_or(1.0) as f32,
                        values[1].as_f64().unwrap_or(1.0) as f32,
                        values[2].as_f64().unwrap_or(1.0) as f32,
                    ]
                })
                .unwrap_or([1.0; 3]);
            let attenuation_distance = material
                .extension_value("KHR_materials_volume")
                .and_then(|value| value.get("attenuationDistance"))
                .and_then(|value| value.as_f64())
                .filter(|distance| *distance > 0.0)
                .map(|distance| distance as f32)
                .unwrap_or(f32::INFINITY);
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
                        brdf_uv: [0.0, 0.0],
                        specular: [0.0; 3],
                        f0: [0.0; 3],
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
                    clearcoat,
                    clearcoat_roughness,
                    transmission,
                    anisotropy,
                    anisotropy_rotation,
                    sheen_color,
                    sheen_roughness,
                    iridescence,
                    iridescence_roughness,
                    iridescence_ior,
                    iridescence_thickness: [thickness_min, thickness_max],
                    thickness,
                    attenuation_color,
                    attenuation_distance,
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
        lights: default_lights().to_vec(),
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

/// 32x32 RGBA8 split-sum BRDF LUT for GPU sampling.
pub fn brdf_lut_bytes() -> Vec<u8> {
    const SIZE: usize = 32;
    let mut bytes = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        let roughness = (y as f32 + 0.5) / SIZE as f32;
        for x in 0..SIZE {
            let n_dot_v = (x as f32 + 0.5) / SIZE as f32;
            let (scale, bias) = brdf_lut(n_dot_v, roughness);
            bytes.extend_from_slice(&[
                (scale * 255.0).clamp(0.0, 255.0) as u8,
                (bias * 255.0).clamp(0.0, 255.0) as u8,
                0,
                255,
            ]);
        }
    }
    bytes
}

pub fn create_brdf_lut_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::TextureView, wgpu::Sampler) {
    let data = brdf_lut_bytes();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("manato brdf lut"),
        size: wgpu::Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(32 * 4),
            rows_per_image: Some(32),
        },
        wgpu::Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("manato brdf lut sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    (texture.create_view(&Default::default()), sampler)
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
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
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
    @location(3) brdf_uv: vec2<f32>,
    @location(4) specular: vec3<f32>,
    @location(5) f0: vec3<f32>,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) brdf_uv: vec2<f32>,
    @location(3) specular: vec3<f32>,
    @location(4) f0: vec3<f32>,
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
    out.brdf_uv = in.brdf_uv;
    out.specular = in.specular;
    out.f0 = in.f0;
    return out;
}
@group(0) @binding(0) var base_color: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var brdf_lut: texture_2d<f32>;
@group(0) @binding(3) var lut_samp: sampler;
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = textureSample(base_color, samp, in.uv) * in.color;
    let uv = in.brdf_uv * 0.96875 + 0.015625;
    let brdf = textureSample(brdf_lut, lut_samp, uv);
    let specular = in.specular * (in.f0 * brdf.r + brdf.g);
    return vec4<f32>(clamp(base.rgb + specular, vec3<f32>(0.0), vec3<f32>(1.0)), base.a);
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
                    wgpu::VertexAttribute {
                        offset: 72,
                        shader_location: 3,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                    wgpu::VertexAttribute {
                        offset: 80,
                        shader_location: 4,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    wgpu::VertexAttribute {
                        offset: 92,
                        shader_location: 5,
                        format: wgpu::VertexFormat::Float32x3,
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
    fn brdf_lut_texture_data_is_populated() {
        let bytes = brdf_lut_bytes();
        assert_eq!(bytes.len(), 32 * 32 * 4);
        assert!(bytes
            .chunks_exact(4)
            .any(|texel| texel[0] > 0 || texel[1] > 0));
    }

    fn shade_with(sample: MaterialSample) -> Shaded {
        shade_color_parts(Vec3::Y, [1.0; 4], None, &default_lights(), &sample)
    }

    fn shade_tilted(sample: MaterialSample) -> Shaded {
        let normal = Vec3::new(0.0, 0.6, 1.0).normalize();
        shade_color_parts(normal, [1.0; 4], None, &default_lights(), &sample)
    }

    #[test]
    fn clearcoat_and_transmission_modify_shading() {
        let base = shade_with(MaterialSample::default());
        let coated = shade_with(MaterialSample {
            clearcoat: 1.0,
            clearcoat_roughness: 0.2,
            ..Default::default()
        });
        let transmitted = shade_with(MaterialSample {
            transmission: 0.8,
            ..Default::default()
        });
        assert_ne!(base.color, coated.color);
        assert!(coated.color.length() > 0.0);
        assert_ne!(transmitted.color, base.color);
    }

    #[test]
    fn anisotropy_modifies_specular_highlight() {
        let base = MaterialSample {
            roughness: 0.35,
            ..Default::default()
        };
        let isotropic = shade_with(base);
        let anisotropic = shade_with(MaterialSample {
            anisotropy: 1.0,
            ..base
        });
        assert_ne!(isotropic.color, anisotropic.color);
        assert_eq!(anisotropic.specular, Vec3::ZERO);
    }

    #[test]
    fn sheen_adds_a_cloth_lobe() {
        let plain = shade_with(MaterialSample {
            roughness: 0.4,
            ..Default::default()
        });
        let sheen = shade_with(MaterialSample {
            roughness: 0.4,
            sheen_color: [1.0, 0.4, 0.6],
            sheen_roughness: 0.2,
            ..Default::default()
        });
        assert_ne!(plain.color, sheen.color);
    }

    #[test]
    fn iridescence_tints_specular() {
        let plain = shade_tilted(MaterialSample {
            roughness: 0.2,
            ..Default::default()
        });
        let iridescent = shade_tilted(MaterialSample {
            roughness: 0.2,
            iridescence: 1.0,
            iridescence_roughness: 0.0,
            iridescence_ior: 1.8,
            iridescence_thickness: [300.0, 700.0],
            ..Default::default()
        });
        assert_ne!(plain.color, iridescent.color);
        // Zero strength must be a no-op.
        let off = shade_tilted(MaterialSample {
            roughness: 0.2,
            iridescence: 0.0,
            iridescence_thickness: [300.0, 700.0],
            ..Default::default()
        });
        assert_eq!(plain.color, off.color);
    }

    #[test]
    fn volume_attenuates_transmission() {
        let thin = shade_with(MaterialSample {
            transmission: 1.0,
            thickness: 0.0,
            attenuation_color: [0.1, 0.2, 0.3],
            attenuation_distance: 1.0,
            ..Default::default()
        });
        let thick = shade_with(MaterialSample {
            transmission: 1.0,
            thickness: 2.0,
            attenuation_color: [0.1, 0.2, 0.3],
            attenuation_distance: 1.0,
            ..Default::default()
        });
        assert_ne!(thin.color, thick.color);
        assert!(thick.color.length() < thin.color.length());
        // Volume without transmission is invisible.
        let opaque = shade_with(MaterialSample::default());
        let opaque_volume = shade_with(MaterialSample {
            thickness: 2.0,
            attenuation_color: [0.1, 0.2, 0.3],
            attenuation_distance: 1.0,
            ..Default::default()
        });
        assert_eq!(opaque.color, opaque_volume.color);
    }

    #[test]
    fn volume_transmittance_is_beer_lambert() {
        assert_eq!(volume_transmittance(1.0, [1.0; 3], 1.0), Vec3::ONE);
        let half = volume_transmittance(1.0, [0.5; 3], 1.0);
        assert!((half.x - 0.5).abs() < 1e-5);
        let doubled = volume_transmittance(2.0, [0.5; 3], 1.0);
        assert!((doubled.x - 0.25).abs() < 1e-5);
        assert_eq!(
            volume_transmittance(5.0, [0.5; 3], f32::INFINITY),
            Vec3::ONE
        );
    }

    #[test]
    fn iridescence_tint_varies_with_view_angle() {
        let thickness = [200.0, 600.0];
        let grazing = iridescence_tint(0.05, 1.5, thickness);
        let facing = iridescence_tint(0.95, 1.5, thickness);
        assert_ne!(grazing, facing);
        for channel in grazing.to_array() {
            assert!((0.0..=1.0).contains(&channel));
        }
    }

    #[test]
    fn content_lights_replace_the_default_rig() {
        let mut asset = ModelAsset::default();
        assert_eq!(asset.lights.len(), 2);
        asset.set_lights(&[manato_formats::stage::ModelLightDef {
            direction: [1.0, 0.0, 0.0],
            color: [1.0, 0.0, 0.0],
            intensity: 4.0,
        }]);
        assert_eq!(asset.lights.len(), 1);
        assert_eq!(asset.lights[0].color, Vec3::X);
        asset.set_lights(&[]);
        assert_eq!(asset.lights.len(), 1, "empty config keeps the previous rig");
    }

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
    fn multiple_directional_lights_contribute() {
        let key = default_lights()[0];
        let fill = default_lights()[1];
        let normal = fill.direction;
        let one = shade_color_with(normal, [1.0; 4], 0.0, 0.5, 1.0, None, &[key]);
        let two = shade_color_with(normal, [1.0; 4], 0.0, 0.5, 1.0, None, &[key, fill]);
        assert!(two[0] > one[0], "fill light adds energy");
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
  "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "baseColorFactor": [1,1,1,1]}, "normalTexture": {"index": 1}, "emissiveTexture": {"index": 2}, "emissiveFactor": [1,0,0], "occlusionTexture": {"index": 3}, "extensions": {"KHR_materials_clearcoat": {"clearcoatFactor": 0.8, "clearcoatRoughnessFactor": 0.2}, "KHR_materials_transmission": {"transmissionFactor": 0.3}, "KHR_materials_anisotropy": {"anisotropyStrength": 0.6, "anisotropyRotation": 0.5}, "KHR_materials_sheen": {"sheenColorFactor": [1,0.4,0.6], "sheenRoughnessFactor": 0.2}, "KHR_materials_iridescence": {"iridescenceFactor": 0.9, "iridescenceRoughness": 0.15, "iridescenceIor": 1.7, "iridescenceThicknessMinimum": 200, "iridescenceThicknessMaximum": 600}, "KHR_materials_volume": {"thicknessFactor": 0.5, "attenuationColor": [0.2,0.9,0.4], "attenuationDistance": 1.5}}}],
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
        assert_eq!(draw.clearcoat, 0.8);
        assert_eq!(draw.clearcoat_roughness, 0.2);
        assert_eq!(draw.transmission, 0.3);
        assert_eq!(draw.anisotropy, 0.6);
        assert_eq!(draw.anisotropy_rotation, 0.5);
        assert_eq!(draw.sheen_color, [1.0, 0.4, 0.6]);
        assert_eq!(draw.sheen_roughness, 0.2);
        assert_eq!(draw.iridescence, 0.9);
        assert_eq!(draw.iridescence_roughness, 0.15);
        assert_eq!(draw.iridescence_ior, 1.7);
        assert_eq!(draw.iridescence_thickness, [200.0, 600.0]);
        assert_eq!(draw.thickness, 0.5);
        assert_eq!(draw.attenuation_color, [0.2, 0.9, 0.4]);
        assert_eq!(draw.attenuation_distance, 1.5);
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
