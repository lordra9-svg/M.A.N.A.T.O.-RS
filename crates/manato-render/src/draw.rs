//! Immediate-mode quad batching on top of wgpu.

use bytemuck::{Pod, Zeroable};

use crate::text::{FontAtlas, GLYPH_H, GLYPH_W};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blend {
    Normal,
    Add,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TexRef {
    Atlas(u32),
    Font,
    Solid,
    Shadow,
}

#[derive(Debug, Clone, Copy)]
pub struct DrawRange {
    pub start: u32,
    pub count: u32,
    pub tex: TexRef,
    pub blend: Blend,
}

/// Accumulates quads and the state changes needed to draw them.
#[derive(Default)]
pub struct Painter {
    pub vertices: Vec<Vertex>,
    pub ranges: Vec<DrawRange>,
}

impl Painter {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.ranges.clear();
    }

    fn range(&mut self, tex: TexRef, blend: Blend) {
        if let Some(last) = self.ranges.last_mut() {
            if last.tex == tex
                && last.blend == blend
                && last.start + last.count == self.vertices.len() as u32
            {
                last.count += 6;
                return;
            }
        }
        self.ranges.push(DrawRange {
            start: self.vertices.len() as u32,
            count: 6,
            tex,
            blend,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn quad(
        &mut self,
        tex: TexRef,
        blend: Blend,
        dest: [f32; 4],
        uv: [f32; 4],
        flip_h: bool,
        flip_v: bool,
        color: [f32; 4],
    ) {
        let [x0, y0, x1, y1] = dest;
        let [u0, v0, u1, v1] = uv;
        let (u0, u1) = if flip_h { (u1, u0) } else { (u0, u1) };
        let (v0, v1) = if flip_v { (v1, v0) } else { (v0, v1) };
        let corners = [
            ([x0, y0], [u0, v0]),
            ([x1, y0], [u1, v0]),
            ([x1, y1], [u1, v1]),
            ([x0, y0], [u0, v0]),
            ([x1, y1], [u1, v1]),
            ([x0, y1], [u0, v1]),
        ];
        self.range(tex, blend);
        for (pos, uv) in corners {
            self.vertices.push(Vertex { pos, uv, color });
        }
    }

    pub fn rect(&mut self, blend: Blend, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.quad(
            TexRef::Solid,
            blend,
            [x, y, x + w, y + h],
            [0.0, 0.0, 1.0, 1.0],
            false,
            false,
            color,
        );
    }

    pub fn text(
        &mut self,
        font: &FontAtlas,
        text: &str,
        x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
    ) -> f32 {
        let mut cursor = x;
        for ch in text.chars() {
            if ch == ' ' {
                cursor += GLYPH_W * scale;
                continue;
            }
            let uv = *font.glyph(ch);
            self.quad(
                TexRef::Font,
                Blend::Normal,
                [cursor, y, cursor + GLYPH_W * scale, y + GLYPH_H * scale],
                uv,
                false,
                false,
                color,
            );
            cursor += GLYPH_W * scale;
        }
        cursor - x
    }

    pub fn text_centered(
        &mut self,
        font: &FontAtlas,
        text: &str,
        center_x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
    ) {
        let width = text.chars().count() as f32 * GLYPH_W * scale;
        self.text(
            font,
            text,
            (center_x - width * 0.5).round(),
            y,
            scale,
            color,
        );
    }

    pub fn text_width(text: &str, scale: f32) -> f32 {
        text.chars().count() as f32 * GLYPH_W * scale
    }
}

pub fn create_sprite_shader(device: &wgpu::Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("manato sprite shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct VsIn {
    @location(0) pos: vec2<f32>,
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
    // The game render target is always 320x240 logical pixels.
    out.pos = vec4<f32>(
        in.pos.x / 320.0 * 2.0 - 1.0,
        1.0 - in.pos.y / 240.0 * 2.0,
        0.0,
        1.0,
    );
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv) * in.color;
}
"#
            .into(),
        ),
    })
}

pub fn create_blit_shader(device: &wgpu::Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("manato blit shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct Blit {
    // x0, y0, x1, y1 in normalized device coordinates.
    rect: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> blit: Blit;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let c = corners[index];
    var out: VsOut;
    out.pos = vec4<f32>(
        mix(blit.rect.x, blit.rect.z, c.x),
        mix(blit.rect.y, blit.rect.w, c.y),
        0.0,
        1.0,
    );
    out.uv = c;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src, samp, in.uv);
}
"#
            .into(),
        ),
    })
}

pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 8,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    }
}

pub fn blend_state(blend: Blend) -> wgpu::BlendState {
    match blend {
        Blend::Normal => wgpu::BlendState::ALPHA_BLENDING,
        Blend::Add => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        },
        Blend::Sub => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::ReverseSubtract,
            },
            alpha: wgpu::BlendComponent::OVER,
        },
    }
}
