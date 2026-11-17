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
    animation: Option<ModelAnimation>,
    world0: Vec<Mat4>,
    placement: Option<Placement>,
}

impl ModelAsset {
    /// Vertex data for every draw at `time` seconds into the first animation.
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
                        out.color =
                            shade_color(world_normal, out.color, draw.metallic, draw.roughness);
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
        let Some(animation) = &self.animation else {
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
        if let Some(animation) = &self.animation {
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

/// Simple directional + ambient model lighting. Metallic surfaces reduce the
/// diffuse term and add a small roughness-driven highlight.
fn shade_color(normal: Vec3, base: [f32; 4], metallic: f32, roughness: f32) -> [f32; 4] {
    let normal = if normal.length_squared() > 0.0 {
        normal.normalize()
    } else {
        Vec3::Y
    };
    let light = Vec3::new(0.3, 0.8, 0.5).normalize();
    let diffuse = normal.dot(light).max(0.0);
    let metallic = metallic.clamp(0.0, 1.0);
    let roughness = roughness.clamp(0.0, 1.0);
    let ambient = 0.35;
    let diffuse_strength = 1.0 - metallic * 0.6;
    let specular = (1.0 - roughness) * 0.25 * diffuse.powi(8);
    let light = ambient + 0.6 * diffuse * diffuse_strength + specular;
    [
        (base[0] * light).clamp(0.0, 1.0),
        (base[1] * light).clamp(0.0, 1.0),
        (base[2] * light).clamp(0.0, 1.0),
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
                });
            }
        }
    }
    if draws.is_empty() {
        anyhow::bail!("glTF has no triangle geometry");
    }

    let animation = document.animations().next().and_then(|animation| {
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
                        gltf::animation::util::MorphTargetWeights::F32(values) => values.collect(),
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
                tracks,
                weight_tracks,
            })
        }
    });

    let mut asset = ModelAsset {
        draws,
        nodes,
        animation,
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
    fn directional_lighting_uses_normals_and_pbr_factors() {
        let lit = shade_color(Vec3::new(0.0, 0.8, 0.5).normalize(), [1.0; 4], 0.0, 1.0);
        assert!(lit[0] < 1.0 && lit[0] > 0.35);
        let unlit = shade_color(Vec3::new(0.0, -1.0, 0.0), [1.0; 4], 0.0, 1.0);
        assert!((unlit[0] - 0.35).abs() < 1e-5);
        let metallic = shade_color(Vec3::new(0.0, 0.8, 0.5).normalize(), [1.0; 4], 1.0, 1.0);
        assert!(metallic[0] < lit[0]);
        let glossy = shade_color(Vec3::new(0.0, 0.8, 0.5).normalize(), [1.0; 4], 0.0, 0.0);
        assert!(glossy[0] > lit[0]);
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
  "images": [{"uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}],
  "samplers": [{}],
  "textures": [{"source": 0, "sampler": 0}],
  "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "baseColorFactor": [1,1,1,1]}}],
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
