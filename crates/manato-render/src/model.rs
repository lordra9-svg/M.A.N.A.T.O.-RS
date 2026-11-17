//! Minimal static glTF model rendering with base-color textures.
//!
//! Triangle meshes are baked into screen space with a fixed placement and drawn
//! with `baseColorFactor * baseColorTexture`. Skinning, animations, PBR
//! lighting, and environment maps are not modeled.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use crate::{GAME_HEIGHT, GAME_WIDTH};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModelVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct ModelTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ModelDraw {
    pub vertices: Vec<ModelVertex>,
    pub texture: Option<ModelTexture>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelAsset {
    pub draws: Vec<ModelDraw>,
}

pub fn load(path: &std::path::Path) -> anyhow::Result<ModelAsset> {
    let (document, buffers, images) = gltf::import(path)?;
    let mut draws = Vec::new();
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
            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|coords| coords.into_f32().collect())
                .unwrap_or_default();
            let material = primitive.material();
            let color = material.pbr_metallic_roughness().base_color_factor();
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
            let mut triangles: Vec<([f32; 3], [f32; 2])> = Vec::new();
            let mut push = |index: usize| {
                if let Some(position) = positions.get(index) {
                    let uv = uvs.get(index).copied().unwrap_or([0.0, 0.0]);
                    triangles.push((*position, uv));
                }
            };
            if let Some(indices) = reader.read_indices() {
                let indices: Vec<u32> = indices.into_u32().collect();
                for triangle in indices.chunks_exact(3) {
                    for &index in triangle {
                        push(index as usize);
                    }
                }
            } else {
                for index in 0..positions.len() {
                    push(index);
                }
            }
            if triangles.is_empty() {
                continue;
            }
            let mut vertices = Vec::with_capacity(triangles.len());
            for (position, uv) in triangles {
                vertices.push((position, uv));
            }
            draws.push(ModelDraw {
                vertices: Vec::new(),
                texture,
            });
            draws.last_mut().expect("draw").vertices = vertices
                .into_iter()
                .map(|(position, uv)| ModelVertex {
                    pos: [position[0], position[1], position[2]],
                    uv,
                    color,
                })
                .collect();
        }
    }
    if draws.is_empty() {
        anyhow::bail!("glTF has no triangle geometry");
    }
    // Normalize all draws together so multi-mesh models line up.
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for draw in &draws {
        for vertex in &draw.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.pos[axis]);
                max[axis] = max[axis].max(vertex.pos[axis]);
            }
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
    for draw in &mut draws {
        for vertex in &mut draw.vertices {
            vertex.pos = [
                GAME_WIDTH as f32 * 0.5 + (vertex.pos[0] - center[0]) * scale,
                ground - (vertex.pos[1] - min[1]) * scale,
                0.98,
            ];
        }
    }
    Ok(ModelAsset { draws })
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
        usage: wgpu::BufferUsages::VERTEX,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(asset.draws[0].vertices.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
