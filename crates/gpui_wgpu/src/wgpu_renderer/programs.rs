//! Pipelines for user programs: GPUI's quad pipeline specialized per shader
//! paint, and custom pipelines.
//!
//! Each program links into one shader module (see [`gpui_render::link`]);
//! its pipelines are cached on first use, retaining recently used programs.
//! User data travels in the frame's instance buffer: shader paints bind it a second time
//! as `array<vec4<f32>>` in group 2, and custom pipelines read it as their
//! group-1 data.

use std::{hash::Hash, num::NonZeroU64};

use collections::FxHashMap;
use gpui::{
    PipelineDraw, Quad, ShaderQuad,
    shader::{Pipeline, Program, Topology},
};
use gpui_render::{link, shaders::interface as shader};

use super::{
    WgpuRenderer,
    buffers::InstanceUpload,
    frame,
    pipelines::{self, WgpuBindGroupLayouts, WgpuRenderPipeline},
};

/// Cache budget; programs used in the current frame are always retained.
const RETAINED_PROGRAMS: usize = 64;

/// Pipelines by program, remembering the frame each was last used in.
struct Cache<K, V> {
    entries: FxHashMap<K, (V, u64)>,
}

impl<K: Copy + Eq + Hash + Ord, V> Cache<K, V> {
    fn new() -> Self {
        Self {
            entries: FxHashMap::default(),
        }
    }

    fn use_or_insert(&mut self, key: K, frame: u64, create: impl FnOnce() -> V) {
        self.entries
            .entry(key)
            .or_insert_with(|| (create(), frame))
            .1 = frame;
    }

    fn get(&self, key: &K) -> &V {
        &self.entries[key].0
    }

    /// Drop the least recently used programs that the current frame left idle.
    fn evict(&mut self, frame: u64) {
        let excess = self.entries.len().saturating_sub(RETAINED_PROGRAMS);
        if excess == 0 {
            return;
        }
        let mut idle: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, (_, used))| *used != frame)
            .map(|(key, (_, used))| (*used, *key))
            .collect();
        idle.sort_unstable();
        for (_, key) in idle.into_iter().take(excess) {
            self.entries.remove(&key);
        }
    }
}

pub(super) struct ProgramPipelines {
    shader_quads: Cache<u64, [WgpuRenderPipeline; 2]>,
    custom: Cache<(u64, Topology), wgpu::RenderPipeline>,
    /// Group-2 layouts for shader paints, without and with the backdrop.
    paint_layouts: [wgpu::BindGroupLayout; 2],
    shader_quad_layouts: [wgpu::PipelineLayout; 2],
    custom_layout: wgpu::PipelineLayout,
    target: wgpu::ColorTargetState,
    frame: u64,
}

impl ProgramPipelines {
    pub(super) fn new(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        target: wgpu::ColorTargetState,
    ) -> Self {
        let paint_layout = |backdrop: bool| {
            let fragment = |binding, ty| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty,
                count: None,
            };
            let mut entries = vec![fragment(
                link::bindings::PARAMS,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
            )];
            if backdrop {
                entries.push(fragment(
                    link::bindings::BACKDROP,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ));
                entries.push(fragment(
                    link::bindings::BACKDROP_SAMPLER,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ));
            }
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shader_paint_layout"),
                entries: &entries,
            })
        };
        let paint_layouts = [paint_layout(false), paint_layout(true)];
        let pipeline_layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let groups = |paint| [Some(&layouts.globals), Some(&layouts.instances), paint];
        let shader_quad_layouts = paint_layouts
            .each_ref()
            .map(|paint| pipeline_layout("shader_quad_layout", &groups(Some(paint))));
        let custom_layout = pipeline_layout("custom_pipeline_layout", &groups(None)[..2]);
        Self {
            shader_quads: Cache::new(),
            custom: Cache::new(),
            paint_layouts,
            shader_quad_layouts,
            custom_layout,
            target,
            frame: 0,
        }
    }

    pub(super) fn paint_layout(&self, backdrop: bool) -> &wgpu::BindGroupLayout {
        &self.paint_layouts[usize::from(backdrop)]
    }

    /// Create the pipelines a scene needs, and evict programs it left unused.
    pub(super) fn prepare(&mut self, device: &wgpu::Device, scene: &gpui::Scene) {
        self.frame += 1;
        let frame = self.frame;
        for quad in &scene.shader_quads {
            let program = &quad.program;
            let layout = &self.shader_quad_layouts[usize::from(program.uses_backdrop())];
            let target = &self.target;
            self.shader_quads.use_or_insert(program.id(), frame, || {
                specialize_quads(device, layout, target, program)
            });
        }
        for draw in &scene.pipeline_draws {
            let pipeline = &draw.pipeline;
            let (layout, target) = (&self.custom_layout, &self.target);
            self.custom
                .use_or_insert((pipeline.id(), pipeline.topology()), frame, || {
                    create_custom(device, layout, target, pipeline)
                });
        }
        self.shader_quads.evict(frame);
        self.custom.evict(frame);
    }
}

fn specialize_quads(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    target: &wgpu::ColorTargetState,
    program: &Program,
) -> [WgpuRenderPipeline; 2] {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shader_quad"),
        source: wgpu::ShaderSource::Wgsl(link::shader_quad_module(program).into()),
    });
    [shader::QUADS, shader::SMOOTHED_QUADS].map(|specification| {
        pipelines::create_render_pipeline(device, specification, layout, target, 1, &module)
    })
}

fn create_custom(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    target: &wgpu::ColorTargetState,
    pipeline: &Pipeline,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("custom_pipeline"),
        source: wgpu::ShaderSource::Wgsl(link::pipeline_module(pipeline).into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("custom_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("pipeline_vertex"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("pipeline_fragment"),
            targets: &[Some(target.clone())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: match pipeline.topology() {
                Topology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
                Topology::TriangleList => wgpu::PrimitiveTopology::TriangleList,
            },
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl WgpuRenderer {
    /// Whether this renderer runs shader paints and custom pipelines: their
    /// data needs fragment-stage storage buffers, which the downlevel tier lacks.
    pub fn supports_shader_paint(&self) -> bool {
        self.resources
            .as_ref()
            .is_some_and(|resources| resources.renderer_tier == crate::RendererTier::Modern)
    }

    pub(super) fn draw_shader_quads(
        &self,
        quads: &[ShaderQuad],
        smoothed: bool,
        backdrop: Option<&wgpu::TextureView>,
        instances: &mut InstanceUpload,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> frame::DrawResult {
        let slots = quads.iter().map(ShaderQuad::parameter_slots).sum();
        let params = instances
            .write_iter(slots, quads.iter().flat_map(ShaderQuad::parameters))
            .ok_or(frame::DrawError::CapacityPlanningInvariant)?;
        let bound = quads.iter().scan(params.range().start, |next, quad| {
            let bound = quad.bind(*next);
            *next += quad.parameter_slots() as u32;
            Some(bound)
        });
        let quads_slice = instances
            .write_iter::<Quad>(quads.len(), bound)
            .ok_or(frame::DrawError::CapacityPlanningInvariant)?;

        let resources = self.resources();
        let program = &quads[0].program;
        let backdrop = match (program.uses_backdrop(), backdrop) {
            (false, _) => None,
            (true, Some(view)) => Some((view, &resources.surface_sampler)),
            (true, None) => return Err(frame::DrawError::MissingIntermediateTarget),
        };
        let programs = &resources.programs;
        let paint_group = resources.instances.paint_bind_group(
            &resources.device,
            programs.paint_layout(program.uses_backdrop()),
            backdrop,
        );
        pass.set_bind_group(link::bindings::GROUP, &paint_group, &[]);
        let pipeline = &programs.shader_quads.get(&program.id())[usize::from(smoothed)];
        pass.set_pipeline(pipeline);
        quads_slice.set_data_bind_group(pass, resources.instances.bind_group());
        pass.draw(0..pipeline.fixed_vertex_count(), quads_slice.range());
        Ok(())
    }

    pub(super) fn draw_pipelines(
        &self,
        draws: &[PipelineDraw],
        instances: &mut InstanceUpload,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> frame::DrawResult {
        let headers = instances
            .write_iter(
                PipelineDraw::HEADER_SLOTS * draws.len(),
                draws.iter().flat_map(PipelineDraw::header),
            )
            .ok_or(frame::DrawError::CapacityPlanningInvariant)?;
        let first_header = headers.range().start;
        let pipeline = &draws[0].pipeline;
        let count = draws.iter().map(|draw| draw.count).sum::<u32>();
        let blocks = draws.iter().enumerate().flat_map(|(index, draw)| {
            draw.instance_blocks(first_header + (PipelineDraw::HEADER_SLOTS * index) as u32)
        });
        let first = instances
            .write_blocks(pipeline.instance_slots() + 1, count, blocks)
            .ok_or(frame::DrawError::CapacityPlanningInvariant)?;

        let resources = self.resources();
        pass.set_pipeline(
            resources
                .programs
                .custom
                .get(&(pipeline.id(), pipeline.topology())),
        );
        pass.set_bind_group(
            shader::DATA_BIND_GROUP,
            resources.instances.bind_group(),
            &[],
        );
        pass.draw(0..pipeline.vertex_count(), first..first + count);
        Ok(())
    }
}
