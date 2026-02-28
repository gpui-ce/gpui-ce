use naga::back::msl;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{self as metal, MTLDevice, MTLLibrary};

fn map_blend_factor(factor: crate::BlendFactor) -> metal::MTLBlendFactor {
    use crate::BlendFactor as Bf;
    use metal::MTLBlendFactor as Mbf;

    match factor {
        Bf::Zero => Mbf::Zero,
        Bf::One => Mbf::One,
        Bf::Src => Mbf::SourceColor,
        Bf::OneMinusSrc => Mbf::OneMinusSourceColor,
        Bf::Dst => Mbf::DestinationColor,
        Bf::OneMinusDst => Mbf::OneMinusDestinationColor,
        Bf::SrcAlpha => Mbf::SourceAlpha,
        Bf::OneMinusSrcAlpha => Mbf::OneMinusSourceAlpha,
        Bf::DstAlpha => Mbf::DestinationAlpha,
        Bf::OneMinusDstAlpha => Mbf::OneMinusDestinationAlpha,
        Bf::Constant => Mbf::BlendColor,
        Bf::OneMinusConstant => Mbf::OneMinusBlendColor,
        Bf::SrcAlphaSaturated => Mbf::SourceAlphaSaturated,
    }
}

fn map_blend_op(operation: crate::BlendOperation) -> metal::MTLBlendOperation {
    use crate::BlendOperation as Bo;
    use metal::MTLBlendOperation as Mbo;

    match operation {
        Bo::Add => Mbo::Add,
        Bo::Subtract => Mbo::Subtract,
        Bo::ReverseSubtract => Mbo::ReverseSubtract,
        Bo::Min => Mbo::Min,
        Bo::Max => Mbo::Max,
    }
}

fn map_blend_component(
    component: &crate::BlendComponent,
) -> (
    metal::MTLBlendOperation,
    metal::MTLBlendFactor,
    metal::MTLBlendFactor,
) {
    (
        map_blend_op(component.operation),
        map_blend_factor(component.src_factor),
        map_blend_factor(component.dst_factor),
    )
}

fn map_stencil_op(op: crate::StencilOperation) -> metal::MTLStencilOperation {
    use crate::StencilOperation as So;
    use metal::MTLStencilOperation as Mso;

    match op {
        So::Keep => Mso::Keep,
        So::Zero => Mso::Zero,
        So::Replace => Mso::Replace,
        So::IncrementClamp => Mso::IncrementClamp,
        So::IncrementWrap => Mso::IncrementWrap,
        So::DecrementClamp => Mso::DecrementClamp,
        So::DecrementWrap => Mso::DecrementWrap,
        So::Invert => Mso::Invert,
    }
}

fn create_stencil_desc(
    face: &crate::StencilFaceState,
    read_mask: u32,
    write_mask: u32,
) -> Retained<metal::MTLStencilDescriptor> {
    let desc = unsafe { metal::MTLStencilDescriptor::new() };
    desc.setStencilCompareFunction(super::map_compare_function(face.compare));
    desc.setReadMask(read_mask);
    desc.setWriteMask(write_mask);
    desc.setStencilFailureOperation(map_stencil_op(face.fail_op));
    desc.setDepthFailureOperation(map_stencil_op(face.depth_fail_op));
    desc.setDepthStencilPassOperation(map_stencil_op(face.pass_op));
    desc
}

fn create_depth_stencil_desc(
    state: &crate::DepthStencilState,
) -> Retained<metal::MTLDepthStencilDescriptor> {
    let desc = unsafe { metal::MTLDepthStencilDescriptor::new() };
    desc.setDepthCompareFunction(super::map_compare_function(state.depth_compare));
    desc.setDepthWriteEnabled(state.depth_write_enabled);

    let s = &state.stencil;
    if s.front != crate::StencilFaceState::IGNORE {
        let face_desc = create_stencil_desc(&s.front, s.read_mask, s.write_mask);
        desc.setFrontFaceStencil(Some(&face_desc));
    }
    if s.back != crate::StencilFaceState::IGNORE {
        let face_desc = create_stencil_desc(&s.back, s.read_mask, s.write_mask);
        desc.setBackFaceStencil(Some(&face_desc));
    }

    desc
}

struct CompiledShader {
    library: Retained<ProtocolObject<dyn metal::MTLLibrary>>,
    function: Retained<ProtocolObject<dyn metal::MTLFunction>>,
    attribute_mappings: Vec<crate::VertexAttributeMapping>,
    wg_size: metal::MTLSize,
    wg_memory_sizes: Vec<u32>,
}

bitflags::bitflags! {
    #[repr(transparent)]
    struct ShaderFlags: u32 {
        const ALLOW_POINT_SIZE = 1<<0;
    }
}

fn _align_to(offset: u32, alignment: u32) -> u32 {
    let remain = offset % alignment;
    if remain != 0 {
        offset + alignment - remain
    } else {
        offset
    }
}

fn make_pipeline_layout(
    bind_group_layouts: &[&crate::ShaderDataLayout],
    reserved_vertex_buffers: u32,
) -> super::PipelineLayout {
    let mut group_mappings = Vec::with_capacity(bind_group_layouts.len());
    let mut unsized_buffer_count = 0;
    let mut num_textures = 0u32;
    let mut num_samplers = 0u32;
    let mut num_buffers = reserved_vertex_buffers;
    for layout in bind_group_layouts.iter() {
        let mut targets = Vec::with_capacity(layout.bindings.len());
        for &(_, ref binding) in layout.bindings.iter() {
            targets.push(match *binding {
                crate::ShaderBinding::Texture => {
                    num_textures += 1;
                    num_textures - 1
                }
                crate::ShaderBinding::Sampler => {
                    num_samplers += 1;
                    num_samplers - 1
                }
                crate::ShaderBinding::Buffer => {
                    unsized_buffer_count += 1;
                    num_buffers += 1;
                    num_buffers - 1
                }
                crate::ShaderBinding::TextureArray { .. }
                | crate::ShaderBinding::BufferArray { .. } => unimplemented!(),
                crate::ShaderBinding::AccelerationStructure => {
                    num_buffers += 1;
                    num_buffers - 1
                }
                crate::ShaderBinding::Plain { .. } => {
                    num_buffers += 1;
                    num_buffers - 1
                }
            });
        }

        group_mappings.push(super::ShaderDataMapping {
            visibility: crate::ShaderVisibility::empty(),
            targets: targets.into_boxed_slice(),
        });
    }

    super::PipelineLayout {
        group_mappings: group_mappings.into_boxed_slice(),
        group_infos: bind_group_layouts
            .iter()
            .map(|layout| layout.to_info())
            .collect(),
        sizes_buffer_slot: if unsized_buffer_count != 0 {
            Some(num_buffers)
        } else {
            None
        },
    }
}

impl super::Context {
    fn load_shader(
        &self,
        sf: crate::ShaderFunction,
        bind_group_layouts: &[&crate::ShaderDataLayout],
        vertex_fetch_states: &[crate::VertexFetchState],
        pipeline_layout: &mut super::PipelineLayout,
        flags: ShaderFlags,
    ) -> CompiledShader {
        let ep_index = sf.entry_point_index();
        let ep = &sf.shader.module.entry_points[ep_index];
        let ep_info = sf.shader.info.get_entry_point(ep_index);
        let _ = sf.shader.source;

        let (mut module, module_info) = sf.shader.resolve_constants(&sf.constants);
        crate::Shader::fill_resource_bindings(
            &mut module,
            &mut pipeline_layout.group_infos,
            ep.stage,
            ep_info,
            bind_group_layouts,
        );
        let attribute_mappings =
            crate::Shader::fill_vertex_locations(&mut module, ep_index, vertex_fetch_states);

        // figure out how much workgroup memory is needed for each binding
        let mut wg_memory_sizes = Vec::new();
        for (var_handle, var) in module.global_variables.iter() {
            if var.space == naga::AddressSpace::WorkGroup && !ep_info[var_handle].is_empty() {
                let size = module.types[var.ty].inner.size(module.to_ctx());
                //TODO: use `u32::next_multiple_of`
                wg_memory_sizes.push(((size - 1) | 0xF) + 1); // multiple of 16
            }
        }

        // copy the visibility for convenience
        for (group_mapping, group_info) in pipeline_layout
            .group_mappings
            .iter_mut()
            .zip(pipeline_layout.group_infos.iter())
        {
            group_mapping.visibility = group_info.visibility;
        }

        let mut naga_resources = msl::EntryPointResources::default();
        if let Some(slot) = pipeline_layout.sizes_buffer_slot {
            naga_resources.sizes_buffer = Some(slot as _);
        }
        for (group_index, (group_layout, group_mapping)) in bind_group_layouts
            .iter()
            .zip(pipeline_layout.group_mappings.iter_mut())
            .enumerate()
        {
            for (binding_index, (&(_, proto), &slot)) in group_layout
                .bindings
                .iter()
                .zip(group_mapping.targets.iter())
                .enumerate()
            {
                let res_binding = naga::ResourceBinding {
                    group: group_index as u32,
                    binding: binding_index as u32,
                };
                let bind_target = match proto {
                    crate::ShaderBinding::Texture => msl::BindTarget {
                        texture: Some(slot as _),
                        ..Default::default()
                    },
                    crate::ShaderBinding::Sampler => msl::BindTarget {
                        sampler: Some(msl::BindSamplerTarget::Resource(slot as _)),
                        ..Default::default()
                    },
                    crate::ShaderBinding::Buffer
                    | crate::ShaderBinding::Plain { .. }
                    | crate::ShaderBinding::AccelerationStructure => msl::BindTarget {
                        buffer: Some(slot as _),
                        ..Default::default()
                    },
                    crate::ShaderBinding::TextureArray { .. }
                    | crate::ShaderBinding::BufferArray { .. } => todo!(),
                };
                naga_resources.resources.insert(res_binding, bind_target);
            }
        }

        let naga_options = msl::Options {
            lang_version: (
                (self.info.language_version.0 as u32 >> 16) as u8,
                self.info.language_version.0 as u8,
            ),
            inline_samplers: Default::default(),
            spirv_cross_compatibility: false,
            fake_missing_bindings: false,
            per_entry_point_map: Some((sf.entry_point.to_string(), naga_resources))
                .into_iter()
                .collect(),
            bounds_check_policies: naga::proc::BoundsCheckPolicies::default(),
            zero_initialize_workgroup_memory: false,
            force_loop_bounding: false,
        };

        let pipeline_options = msl::PipelineOptions {
            allow_and_force_point_size: flags.contains(ShaderFlags::ALLOW_POINT_SIZE),
            vertex_pulling_transform: false,
            vertex_buffer_mappings: Vec::new(),
        };
        let (source, info) =
            msl::write_string(&module, &module_info, &naga_options, &pipeline_options).unwrap();

        log::debug!(
            "Naga generated shader for entry point '{}' and stage {:?}\n{}",
            sf.entry_point,
            ep.stage,
            &source
        );

        let source_string = NSString::from_str(&source);
        let options = metal::MTLCompileOptions::new();
        options.setLanguageVersion(self.info.language_version);
        options.setPreserveInvariance(true);

        let library = self
            .device
            .lock()
            .unwrap()
            .newLibraryWithSource_options_error(&source_string, Some(&options))
            .unwrap_or_else(|err| {
                panic!("MSL compilation error:\n{}", err.localizedDescription());
            });

        let ep = &module.entry_points[ep_index];
        let name = info.entry_point_names[ep_index].as_ref().unwrap();
        let wg_size = metal::MTLSize {
            width: ep.workgroup_size[0] as _,
            height: ep.workgroup_size[1] as _,
            depth: ep.workgroup_size[2] as _,
        };

        let name_string = NSString::from_str(name);
        let function = library.newFunctionWithName(&name_string).unwrap();

        CompiledShader {
            library,
            function,
            attribute_mappings,
            wg_size,
            wg_memory_sizes,
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::ShaderDevice for super::Context {
    type ComputePipeline = super::ComputePipeline;
    type RenderPipeline = super::RenderPipeline;

    fn create_compute_pipeline(&self, desc: crate::ComputePipelineDesc) -> super::ComputePipeline {
        use metal::MTLDevice as _;
        let mut layout = make_pipeline_layout(desc.data_layouts, 0);

        objc2::rc::autoreleasepool(|_| {
            let cs = self.load_shader(
                desc.compute,
                desc.data_layouts,
                &[],
                &mut layout,
                ShaderFlags::empty(),
            );

            //TODO: use `newComputePipelineStateWithDescriptor_options_reflection`
            // https://github.com/madsmtm/objc2/issues/683
            let raw = self
                .device
                .lock()
                .unwrap()
                .newComputePipelineStateWithFunction_error(&cs.function)
                .unwrap();

            super::ComputePipeline {
                raw,
                name: desc.name.to_string(),
                lib: cs.library,
                layout,
                wg_size: cs.wg_size,
                wg_memory_sizes: cs.wg_memory_sizes.into_boxed_slice(),
            }
        })
    }

    fn destroy_compute_pipeline(&self, _pipeline: &mut super::ComputePipeline) {
        //TODO: is there a way to release?
    }

    fn create_render_pipeline(&self, desc: crate::RenderPipelineDesc) -> super::RenderPipeline {
        let mut layout = make_pipeline_layout(desc.data_layouts, desc.vertex_fetches.len() as u32);

        let triangle_fill_mode = match desc.primitive.wireframe {
            false => metal::MTLTriangleFillMode::Fill,
            true => metal::MTLTriangleFillMode::Lines,
        };

        let (primitive_class, primitive_type) = match desc.primitive.topology {
            crate::PrimitiveTopology::PointList => (
                metal::MTLPrimitiveTopologyClass::Point,
                metal::MTLPrimitiveType::Point,
            ),
            crate::PrimitiveTopology::LineList => (
                metal::MTLPrimitiveTopologyClass::Line,
                metal::MTLPrimitiveType::Line,
            ),
            crate::PrimitiveTopology::LineStrip => (
                metal::MTLPrimitiveTopologyClass::Line,
                metal::MTLPrimitiveType::LineStrip,
            ),
            crate::PrimitiveTopology::TriangleList => (
                metal::MTLPrimitiveTopologyClass::Triangle,
                metal::MTLPrimitiveType::Triangle,
            ),
            crate::PrimitiveTopology::TriangleStrip => (
                metal::MTLPrimitiveTopologyClass::Triangle,
                metal::MTLPrimitiveType::TriangleStrip,
            ),
        };

        objc2::rc::autoreleasepool(|_| {
            let descriptor = metal::MTLRenderPipelineDescriptor::new();

            let vs = self.load_shader(
                desc.vertex,
                desc.data_layouts,
                desc.vertex_fetches,
                &mut layout,
                match primitive_class {
                    metal::MTLPrimitiveTopologyClass::Point => ShaderFlags::ALLOW_POINT_SIZE,
                    _ => ShaderFlags::empty(),
                },
            );
            descriptor.setVertexFunction(Some(&vs.function));
            descriptor.setRasterSampleCount(desc.multisample_state.sample_count as _);
            descriptor.setAlphaToCoverageEnabled(desc.multisample_state.alpha_to_coverage);

            // Fragment shader
            let fs_lib = if let Some(desc_fs) = desc.fragment {
                let fs = self.load_shader(
                    desc_fs,
                    desc.data_layouts,
                    &[],
                    &mut layout,
                    ShaderFlags::empty(),
                );
                descriptor.setFragmentFunction(Some(&fs.function));
                Some(fs.library)
            } else {
                None
            };

            let vertex_descriptor = unsafe { metal::MTLVertexDescriptor::new() };
            for (i, vf) in desc.vertex_fetches.iter().enumerate() {
                unsafe {
                    let buffer_desc = vertex_descriptor.layouts().objectAtIndexedSubscript(i);
                    buffer_desc.setStride(vf.layout.stride as usize);
                    buffer_desc.setStepFunction(if vf.instanced {
                        metal::MTLVertexStepFunction::PerInstance
                    } else {
                        metal::MTLVertexStepFunction::PerVertex
                    })
                };
            }
            for mapping in vs.attribute_mappings.into_iter() {
                let vf = &desc.vertex_fetches[mapping.buffer_index];
                let (_, attrib) = vf.layout.attributes[mapping.attribute_index];
                let (vertex_format, _) = super::map_vertex_format(attrib.format);
                unsafe {
                    let attribute_desc = vertex_descriptor
                        .attributes()
                        .objectAtIndexedSubscript(mapping.location as usize);
                    attribute_desc.setFormat(vertex_format);
                    attribute_desc.setBufferIndex(mapping.buffer_index);
                    attribute_desc.setOffset(attrib.offset as usize);
                }
            }
            descriptor.setVertexDescriptor(Some(&vertex_descriptor));

            for (i, ct) in desc.color_targets.iter().enumerate() {
                let at_descriptor =
                    unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(i) };
                at_descriptor.setPixelFormat(super::map_texture_format(ct.format));

                let mut write_mask = metal::MTLColorWriteMask::empty();
                if ct.write_mask.contains(crate::ColorWrites::RED) {
                    write_mask |= metal::MTLColorWriteMask::Red;
                }
                if ct.write_mask.contains(crate::ColorWrites::GREEN) {
                    write_mask |= metal::MTLColorWriteMask::Green;
                }
                if ct.write_mask.contains(crate::ColorWrites::BLUE) {
                    write_mask |= metal::MTLColorWriteMask::Blue;
                }
                if ct.write_mask.contains(crate::ColorWrites::ALPHA) {
                    write_mask |= metal::MTLColorWriteMask::Alpha;
                }
                at_descriptor.setWriteMask(write_mask);

                if let Some(ref blend) = ct.blend {
                    at_descriptor.setBlendingEnabled(true);
                    let (color_op, color_src, color_dst) = map_blend_component(&blend.color);
                    let (alpha_op, alpha_src, alpha_dst) = map_blend_component(&blend.alpha);

                    at_descriptor.setRgbBlendOperation(color_op);
                    at_descriptor.setSourceRGBBlendFactor(color_src);
                    at_descriptor.setDestinationRGBBlendFactor(color_dst);

                    at_descriptor.setAlphaBlendOperation(alpha_op);
                    at_descriptor.setSourceAlphaBlendFactor(alpha_src);
                    at_descriptor.setDestinationAlphaBlendFactor(alpha_dst);
                }
            }

            let depth_stencil = match desc.depth_stencil {
                Some(ref ds) => {
                    let raw_format = super::map_texture_format(ds.format);
                    descriptor.setDepthAttachmentPixelFormat(raw_format);
                    if ds.format.aspects().contains(crate::TexelAspects::STENCIL) {
                        descriptor.setStencilAttachmentPixelFormat(raw_format);
                    }

                    let ds_descriptor = create_depth_stencil_desc(ds);
                    let raw = self
                        .device
                        .lock()
                        .unwrap()
                        .newDepthStencilStateWithDescriptor(&ds_descriptor)
                        .unwrap();
                    Some((raw, ds.bias))
                }
                None => None,
            };

            if !desc.name.is_empty() {
                descriptor.setLabel(Some(&NSString::from_str(desc.name)));
            }

            let raw = self
                .device
                .lock()
                .unwrap()
                .newRenderPipelineStateWithDescriptor_error(&descriptor)
                .unwrap();

            super::RenderPipeline {
                raw,
                name: desc.name.to_string(),
                vs_lib: vs.library,
                fs_lib,
                layout,
                primitive_type,
                triangle_fill_mode,
                front_winding: match desc.primitive.front_face {
                    crate::FrontFace::Cw => metal::MTLWinding::Clockwise,
                    crate::FrontFace::Ccw => metal::MTLWinding::CounterClockwise,
                },
                cull_mode: match desc.primitive.cull_mode {
                    None => metal::MTLCullMode::None,
                    Some(crate::Face::Front) => metal::MTLCullMode::Front,
                    Some(crate::Face::Back) => metal::MTLCullMode::Back,
                },
                depth_clip_mode: if desc.primitive.unclipped_depth {
                    metal::MTLDepthClipMode::Clamp
                } else {
                    metal::MTLDepthClipMode::Clip
                },
                depth_stencil,
            }
        })
    }

    fn destroy_render_pipeline(&self, _pipeline: &mut super::RenderPipeline) {
        //TODO: is there a way to release?
    }
}
