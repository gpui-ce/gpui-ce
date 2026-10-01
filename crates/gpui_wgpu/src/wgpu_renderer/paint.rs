//! Inline, device-owned pipelines and amortized uniform uploads for composed paints.
use collections::FxHashMap;
use gpui::Scene;
use std::{num::NonZeroU64, ops::Range};

const CACHE_LIMIT: usize = 128;
const DRAW_BYTES: u64 = 64;

struct Program {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    parameter_bytes: u64,
    last_used: u64,
}

pub(super) enum PaintError {
    InvalidShader(String),
    UniformCapacity,
    InvalidGeometry,
}

pub(super) struct PaintResources {
    programs: FxHashMap<u64, Program>,
    buffer: wgpu::Buffer,
    capacity: u64,
    offsets: Vec<(u32, u32)>,
    staging: Vec<u8>,
    scissors: Vec<Option<[u32; 4]>>,
    viewport: [u32; 2],
    frame: u64,
    #[cfg(feature = "test-support")]
    pipeline_compilations: u64,
}

impl PaintResources {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            programs: FxHashMap::default(),
            buffer: buffer(device, 256),
            capacity: 256,
            offsets: Vec::new(),
            staging: Vec::new(),
            scissors: Vec::new(),
            viewport: [0; 2],
            frame: 0,
            #[cfg(feature = "test-support")]
            pipeline_compilations: 0,
        }
    }

    #[cfg(feature = "test-support")]
    pub(super) fn diagnostics(&self) -> (usize, u64, u64) {
        (
            self.programs.len(),
            self.pipeline_compilations,
            self.capacity,
        )
    }

    pub(super) fn invalidate(&mut self) {
        self.programs.clear();
    }

    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        globals: &wgpu::BindGroupLayout,
        scene: &Scene,
        viewport: [f32; 2],
        format: wgpu::TextureFormat,
        alpha: wgpu::CompositeAlphaMode,
    ) -> Result<(), PaintError> {
        self.frame = self.frame.wrapping_add(1);
        self.offsets.clear();
        self.staging.clear();
        self.scissors.clear();
        self.viewport = [viewport[0] as u32, viewport[1] as u32];
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        for quad in &scene.shaders {
            let geometry = [
                quad.bounds.origin.x.0,
                quad.bounds.origin.y.0,
                quad.bounds.size.width.0,
                quad.bounds.size.height.0,
                quad.content_mask.bounds.origin.x.0,
                quad.content_mask.bounds.origin.y.0,
                quad.content_mask.bounds.size.width.0,
                quad.content_mask.bounds.size.height.0,
                quad.opacity,
                quad.scale_factor,
            ];
            if geometry.iter().any(|value| !value.is_finite())
                || quad.scale_factor <= 0.
                || !(0. ..=1.).contains(&quad.opacity)
            {
                return Err(PaintError::InvalidGeometry);
            }
            let rect = scissor(
                [
                    quad.bounds.origin.x.0,
                    quad.bounds.origin.y.0,
                    quad.bounds.size.width.0,
                    quad.bounds.size.height.0,
                ],
                [
                    quad.content_mask.bounds.origin.x.0,
                    quad.content_mask.bounds.origin.y.0,
                    quad.content_mask.bounds.size.width.0,
                    quad.content_mask.bounds.size.height.0,
                ],
                viewport,
            );
            self.scissors.push(rect);
            if rect.is_none() {
                self.offsets.push((0, 0));
                continue;
            }
            let parameters = quad.shader.parameter_slots();
            let parameter_bytes = (parameters.len().max(1) as u64) * 16;
            if parameter_bytes > device.limits().max_uniform_buffer_binding_size {
                return Err(PaintError::UniformCapacity);
            }
            let draw_offset = (self.staging.len() as u64).next_multiple_of(alignment);
            let parameter_offset = (draw_offset + DRAW_BYTES).next_multiple_of(alignment);
            let end = parameter_offset
                .checked_add(parameter_bytes)
                .ok_or(PaintError::UniformCapacity)?;
            if end > device.limits().max_buffer_size.min(u64::from(u32::MAX)) {
                return Err(PaintError::UniformCapacity);
            }
            self.staging.resize(end as usize, 0);
            self.offsets
                .push((draw_offset as u32, parameter_offset as u32));
            let bounds = &quad.bounds;
            let clip = &quad.content_mask.bounds;
            let values = [
                bounds.origin.x.0,
                bounds.origin.y.0,
                bounds.size.width.0,
                bounds.size.height.0,
                clip.origin.x.0,
                clip.origin.y.0,
                clip.origin.x.0 + clip.size.width.0,
                clip.origin.y.0 + clip.size.height.0,
                viewport[0],
                viewport[1],
                quad.opacity,
                quad.scale_factor,
                0.,
                0.,
                0.,
                0.,
            ];
            write_floats(&mut self.staging[draw_offset as usize..], &values);
            for (ix, slot) in parameters.iter().enumerate() {
                write_floats(
                    &mut self.staging[parameter_offset as usize + ix * 16..],
                    slot,
                );
            }
        }
        if self.staging.len() as u64 > self.capacity {
            self.capacity = (self.staging.len() as u64)
                .next_power_of_two()
                .min(device.limits().max_buffer_size.min(u64::from(u32::MAX)));
            self.buffer = buffer(device, self.capacity);
            for program in self.programs.values_mut() {
                program.group = bind_group(
                    device,
                    &program.layout,
                    &self.buffer,
                    program.parameter_bytes,
                );
            }
        }
        for (ix, quad) in scene.shaders.iter().enumerate() {
            if self.scissors[ix].is_none() {
                continue;
            }
            let id = quad.shader.program_id();
            if let Some(program) = self.programs.get_mut(&id) {
                program.last_used = self.frame;
                continue;
            }
            let parameter_bytes = quad.shader.parameter_slots().len().max(1) as u64 * 16;
            let source = shader_source(
                quad.shader.wgsl(),
                alpha == wgpu::CompositeAlphaMode::PreMultiplied,
            );
            validate(&source)?;
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("paint uniforms"),
                entries: &[
                    uniform_entry(0, DRAW_BYTES),
                    uniform_entry(1, parameter_bytes),
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("paint pipeline layout"),
                bind_group_layouts: &[Some(globals), Some(&layout)],
                immediate_size: 0,
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("composed paint"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("composed paint"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("paint_vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("paint_fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(super::pipelines::scene_blend_state(alpha)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            let group = bind_group(device, &layout, &self.buffer, parameter_bytes);
            #[cfg(feature = "test-support")]
            {
                self.pipeline_compilations += 1;
            }
            self.programs.insert(
                id,
                Program {
                    pipeline,
                    layout,
                    group,
                    parameter_bytes,
                    last_used: self.frame,
                },
            );
        }
        if self.programs.len() > CACHE_LIMIT {
            // A frame can use more than the cache budget. Collect once so shrinking that
            // working set on the next frame does not repeatedly scan the entire cache.
            let evictions = eviction_candidates(
                self.programs
                    .iter()
                    .map(|(id, program)| (*id, program.last_used)),
                self.programs.len(),
                self.frame,
                CACHE_LIMIT,
            );
            for id in evictions {
                self.programs.remove(&id);
            }
        }
        if !self.staging.is_empty() {
            queue.write_buffer(&self.buffer, 0, &self.staging);
        }
        Ok(())
    }

    pub(super) fn draw(&self, scene: &Scene, range: Range<usize>, pass: &mut wgpu::RenderPass<'_>) {
        for ix in range {
            let Some([x, y, width, height]) = self.scissors[ix] else {
                continue;
            };
            pass.set_scissor_rect(x, y, width, height);
            let program = &self.programs[&scene.shaders[ix].shader.program_id()];
            let (draw, parameters) = self.offsets[ix];
            pass.set_pipeline(&program.pipeline);
            pass.set_bind_group(1, &program.group, &[draw, parameters]);
            pass.draw(0..6, 0..1);
        }
        pass.set_scissor_rect(0, 0, self.viewport[0], self.viewport[1]);
    }
}

/// Keep every program needed by the current frame, then retain the most recently used
/// inactive programs up to the budget. The id makes equally old entries deterministic.
fn eviction_candidates(
    programs: impl Iterator<Item = (u64, u64)>,
    count: usize,
    frame: u64,
    limit: usize,
) -> Vec<u64> {
    let mut inactive: Vec<_> = programs
        .filter(|(_, last_used)| *last_used != frame)
        .collect();
    inactive.sort_unstable_by_key(|(id, last_used)| (*last_used, *id));
    inactive.truncate(count.saturating_sub(limit));
    inactive.into_iter().map(|(id, _)| id).collect()
}

/// Integer scissoring limits fragment work; the shader retains exact fractional clipping.
fn scissor(bounds: [f32; 4], clip: [f32; 4], viewport: [f32; 2]) -> Option<[u32; 4]> {
    if bounds[2] <= 0. || bounds[3] <= 0. || clip[2] <= 0. || clip[3] <= 0. {
        return None;
    }
    let left = bounds[0].max(clip[0]).max(0.);
    let top = bounds[1].max(clip[1]).max(0.);
    let right = (bounds[0] + bounds[2])
        .min(clip[0] + clip[2])
        .min(viewport[0]);
    let bottom = (bounds[1] + bounds[3])
        .min(clip[1] + clip[3])
        .min(viewport[1]);
    if right <= left || bottom <= top {
        return None;
    }
    let x = left.floor() as u32;
    let y = top.floor() as u32;
    Some([x, y, right.ceil() as u32 - x, bottom.ceil() as u32 - y])
}

fn buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("paint uniform arena"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn uniform_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}
fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    parameter_bytes: u64,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("paint uniforms"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: 0,
                    size: NonZeroU64::new(DRAW_BYTES),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: 0,
                    size: NonZeroU64::new(parameter_bytes),
                }),
            },
        ],
    })
}
fn write_floats(destination: &mut [u8], values: &[f32]) {
    for (bytes, value) in destination.chunks_exact_mut(4).zip(values) {
        bytes.copy_from_slice(&value.to_ne_bytes());
    }
}
fn validate(source: &str) -> Result<(), PaintError> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| PaintError::InvalidShader(e.emit_to_string(source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| PaintError::InvalidShader(e.to_string()))?;
    Ok(())
}
fn shader_source(graph: &str, premultiplied: bool) -> String {
    format!(
        r#"
{graph}
struct PaintDraw {{ bounds: vec4<f32>, clip: vec4<f32>, viewport_opacity_scale: vec4<f32>, padding: vec4<f32> }};
@group(1) @binding(0) var<uniform> paint_draw: PaintDraw;
struct PaintVertex {{ @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }};
@vertex fn paint_vertex(@builtin(vertex_index) index: u32) -> PaintVertex {{
    let corners = array<vec2<f32>, 6>(vec2<f32>(0.,0.), vec2<f32>(1.,0.), vec2<f32>(0.,1.), vec2<f32>(0.,1.), vec2<f32>(1.,0.), vec2<f32>(1.,1.));
    let uv = corners[index];
    let point = paint_draw.bounds.xy + uv * paint_draw.bounds.zw;
    let ndc = point / paint_draw.viewport_opacity_scale.xy * 2. - 1.;
    return PaintVertex(vec4<f32>(ndc.x, -ndc.y, 0., 1.), uv);
}}
@fragment fn paint_fragment(vertex: PaintVertex) -> @location(0) vec4<f32> {{
    let point = vertex.position.xy;
    let scale = paint_draw.viewport_opacity_scale.w;
    var color = gpui_paint_fragment(vertex.uv, vertex.uv * paint_draw.bounds.zw / scale, paint_draw.bounds.zw / scale);
    if any(point < paint_draw.clip.xy) || any(point >= paint_draw.clip.zw) {{ discard; }}
    color.a = clamp(color.a, 0., 1.) * paint_draw.viewport_opacity_scale.z;
    {premultiply}
    return color;
}}
"#,
        premultiply = if premultiplied {
            "color = vec4<f32>(color.rgb * color.a, color.a);"
        } else {
            ""
        }
    )
}

impl std::fmt::Display for PaintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidShader(message) => write!(f, "invalid composed paint shader: {message}"),
            Self::InvalidGeometry => {
                f.write_str("paint bounds, clipping, opacity, or scale are invalid")
            }
            Self::UniformCapacity => f.write_str("paint uniforms exceed device limits"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GRAPH: &str = "struct GpuiPaintParameters { values: array<vec4<f32>, 1> }; @group(1) @binding(1) var<uniform> gpui_paint_parameters: GpuiPaintParameters; fn gpui_paint_fragment(uv: vec2<f32>, position: vec2<f32>, size: vec2<f32>) -> vec4<f32> { return gpui_paint_parameters.values[0]; }";
    #[test]
    fn validates_complete_pipeline_for_both_alpha_modes() {
        for premultiplied in [false, true] {
            assert!(validate(&shader_source(GRAPH, premultiplied)).is_ok());
        }
    }
    #[test]
    fn derivatives_are_evaluated_before_nonuniform_clipping() {
        let graph = GRAPH.replace(
            "return gpui_paint_parameters.values[0];",
            "return vec4<f32>(fwidth(uv.x));",
        );
        assert!(validate(&shader_source(&graph, true)).is_ok());
    }
    #[test]
    fn cache_eviction_preserves_active_programs_and_removes_oldest_first() {
        let programs = [(4, 3), (3, 1), (2, 1), (1, 5), (0, 5)];
        assert_eq!(
            eviction_candidates(programs.into_iter(), 5, 5, 3),
            vec![2, 3]
        );
        // The active working set can exceed the budget and must remain drawable.
        assert_eq!(
            eviction_candidates(programs.into_iter(), 5, 5, 1),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn scissors_round_outward_and_intersect_all_bounds() {
        assert_eq!(
            scissor([-2., -2., 10., 10.], [1.25, 2.75, 20., 20.], [6., 7.]),
            Some([1, 2, 5, 5])
        );
        assert_eq!(
            scissor([0., 0., 0., 1.], [0., 0., 10., 10.], [10., 10.]),
            None
        );
        assert_eq!(
            scissor([20., 0., 1., 1.], [0., 0., 10., 10.], [10., 10.]),
            None
        );
    }
    #[test]
    fn invalid_composition_is_an_error_before_gpu_creation() {
        assert!(matches!(
            validate(&shader_source("broken", false)),
            Err(PaintError::InvalidShader(_))
        ));
    }
}
