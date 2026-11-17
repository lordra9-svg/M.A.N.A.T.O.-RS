//! Minimal static glTF 3D model rendering.
//!
//! Loads triangle meshes, bakes a fixed screen-space placement (centered,
//! scaled to fit, standing on the stage ground), and draws them with a flat
//! material color. Skinning, animations, and textures are not modeled.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use crate::{GAME_HEIGHT, GAME_WIDTH};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModelVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

pub struct ModelAsset {
    pub vertices: Vec<ModelVertex>,
}

pub fn load(path: &std::path::Path) -> anyhow::Result<ModelAsset> {
    let (document, buffers, _images) = gltf::import(path)?;
    let mut raw = Vec::new();
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let color = primitive
                .material()
                .pbr_metallic_roughness()
                .base_color_factor();
            let color = [
                color[0].clamp(0.0, 1.0),
                color[1].clamp(0.0, 1.0),
                color[2].clamp(0.0, 1.0),
                color[3].clamp(0.0, 1.0),
            ];
            if let Some(indices) = reader.read_indices() {
                let indices: Vec<u32> = indices.into_u32().collect();
                for triangle in indices.chunks_exact(3) {
                    for &index in triangle {
                        if let Some(position) = positions.get(index as usize) {
                            raw.push((*position, color));
                        }
                    }
                }
            } else {
                for triangle in positions.chunks_exact(3) {
                    for position in triangle {
                        raw.push((*position, color));
                    }
                }
            }
        }
    }
    if raw.is_empty() {
        anyhow::bail!("glTF has no triangle geometry");
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for (position, _) in &raw {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent = (max[0] - min[0])
        .max(max[1] - min[1])
        .max(max[2] - min[2])
        .max(0.0001);
    let scale = (GAME_HEIGHT as f32 * 0.7) / extent;
    let ground = GAME_HEIGHT as f32 * 0.88;
    let vertices = raw
        .into_iter()
        .map(|(position, color)| ModelVertex {
            pos: [
                GAME_WIDTH as f32 * 0.5 + (position[0] - center[0]) * scale,
                ground - (position[1] - min[1]) * scale,
                0.98,
            ],
            color,
        })
        .collect();
    Ok(ModelAsset { vertices })
}

pub fn create_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("manato model shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
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
    out.color = in.color;
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#
            .into(),
        ),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("manato model layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("manato model pipeline"),
        layout: Some(&layout),
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

pub fn upload_model(device: &wgpu::Device, asset: &ModelAsset) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("manato model vertices"),
        contents: bytemuck::cast_slice(&asset.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    })
}
