use glow::HasContext as _;
use naga::back::glsl;

fn separate<T: PartialEq>(mut iter: impl Iterator<Item = T>) -> bool {
    if let Some(first) = iter.next() {
        iter.all(|el| el == first)
    } else {
        false
    }
}

fn conflate<T: PartialEq>(iter: impl Iterator<Item = T> + Clone) -> Box<[T]> {
    if separate(iter.clone()) {
        iter.collect()
    } else {
        iter.take(1).collect()
    }
}

impl super::Context {
    unsafe fn create_pipeline(
        &self,
        shaders: &[crate::ShaderFunction],
        group_layouts: &[&crate::ShaderDataLayout],
        vertex_fetch_states: &[crate::VertexFetchState],
        name: &str,
        extra_flags: glsl::WriterFlags,
    ) -> super::PipelineInner {
        let gl = self.lock();
        let force_explicit_bindings = self
            .capabilities
            .contains(super::Capabilities::BUFFER_STORAGE);
        let mut naga_options = glsl::Options {
            version: glsl::Version::Embedded {
                version: if force_explicit_bindings { 320 } else { 300 },
                is_webgl: cfg!(target_arch = "wasm32"),
            },
            writer_flags: extra_flags | glsl::WriterFlags::ADJUST_COORDINATE_SPACE,
            binding_map: Default::default(),
            zero_initialize_workgroup_memory: false,
        };

        let mut group_mappings = group_layouts
            .iter()
            .map(|layout| super::ShaderDataMapping {
                targets: vec![Vec::new(); layout.bindings.len()].into_boxed_slice(),
            })
            .collect::<Box<[_]>>();
        if force_explicit_bindings {
            let mut num_textures = 0u32;
            let mut num_samplers = 0u32;
            let mut num_buffers = 0u32;
            for (group_index, (data_mapping, &layout)) in group_mappings
                .iter_mut()
                .zip(group_layouts.iter())
                .enumerate()
            {
                for (binding_index, (slot_list, &(_, ref binding))) in data_mapping
                    .targets
                    .iter_mut()
                    .zip(layout.bindings.iter())
                    .enumerate()
                {
                    let target = match *binding {
                        crate::ShaderBinding::Texture => {
                            num_textures += 1;
                            num_textures - 1
                        }
                        crate::ShaderBinding::Sampler => {
                            num_samplers += 1;
                            num_samplers - 1
                        }
                        crate::ShaderBinding::Buffer => {
                            num_buffers += 1;
                            num_buffers - 1
                        }
                        crate::ShaderBinding::TextureArray { .. }
                        | crate::ShaderBinding::BufferArray { .. }
                        | crate::ShaderBinding::AccelerationStructure => unimplemented!(),
                        crate::ShaderBinding::Plain { .. } => {
                            num_buffers += 1;
                            num_buffers - 1
                        }
                    };

                    let rb = naga::ResourceBinding {
                        group: group_index as u32,
                        binding: binding_index as u32,
                    };
                    naga_options.binding_map.insert(rb, target as u8);
                    slot_list.push(target);
                }
            }
            log::info!(
                "Detected {} textures, {} samples, and {} buffers",
                num_textures,
                num_samplers,
                num_buffers
            );
        }

        let program = gl.create_program().unwrap();
        #[cfg(not(target_arch = "wasm32"))]
        if !name.is_empty() && gl.supports_debug() {
            gl.object_label(glow::PROGRAM, std::mem::transmute(program), Some(name));
        }

        let mut baked_shaders = Vec::with_capacity(shaders.len());
        let mut group_infos = group_layouts
            .iter()
            .map(|layout| layout.to_info())
            .collect::<Vec<_>>();
        let mut attributes = Vec::new();

        for &sf in shaders {
            let ep_index = sf.entry_point_index();
            let ep = &sf.shader.module.entry_points[ep_index];
            let _ = sf.shader.source;

            let (mut module, module_info) = sf.shader.resolve_constants(&sf.constants);
            if force_explicit_bindings {
                let ep_info = module_info.get_entry_point(ep_index);
                crate::Shader::fill_resource_bindings(
                    &mut module,
                    &mut group_infos,
                    ep.stage,
                    ep_info,
                    group_layouts,
                );
            }
            let attribute_mappings =
                crate::Shader::fill_vertex_locations(&mut module, ep_index, vertex_fetch_states);

            for mapping in attribute_mappings {
                let vf = &vertex_fetch_states[mapping.buffer_index];
                let (_, attrib) = vf.layout.attributes[mapping.attribute_index];
                attributes.push(super::VertexAttributeInfo {
                    attrib,
                    buffer_index: mapping.buffer_index as u32,
                    stride: vf.layout.stride as i32,
                    instanced: vf.instanced,
                    location: mapping.location,
                });
            }

            let pipeline_options = glsl::PipelineOptions {
                shader_stage: ep.stage,
                entry_point: sf.entry_point.to_string(),
                multiview: None,
            };
            let mut source = String::new();
            let mut writer = glsl::Writer::new(
                &mut source,
                &module,
                &module_info,
                &naga_options,
                &pipeline_options,
                Default::default(),
            )
            .unwrap();
            let reflection = writer.write().unwrap();

            log::debug!(
                "Naga generated shader for entry point '{}' and stage {:?}\n{}",
                sf.entry_point,
                ep.stage,
                &source
            );

            let target = match ep.stage {
                naga::ShaderStage::Vertex => glow::VERTEX_SHADER,
                naga::ShaderStage::Fragment => glow::FRAGMENT_SHADER,
                naga::ShaderStage::Compute => glow::COMPUTE_SHADER,
                _ => panic!("Unsupported shader stage: {:?}", ep.stage),
            };
            let shader = gl.create_shader(target).unwrap();
            gl.shader_source(shader, &source);
            gl.compile_shader(shader);

            let compiled_ok = gl.get_shader_compile_status(shader);
            let msg = gl.get_shader_info_log(shader);
            assert!(compiled_ok, "Compile: {}", msg);

            gl.attach_shader(program, shader);
            baked_shaders.push((shader, reflection));
        }

        gl.link_program(program);
        log::info!("\tLinked program {:?}", program);

        let linked_ok = gl.get_program_link_status(program);
        let msg = gl.get_program_info_log(program);
        assert!(linked_ok, "Link: {}", msg);
        gl.use_program(Some(program));

        if !force_explicit_bindings {
            let force_uniform_block_assignment = true;
            let mut variables_to_bind = Vec::new();
            for (sf, &(_, ref reflection)) in shaders.iter().zip(baked_shaders.iter()) {
                for (glsl_name, mapping) in reflection.texture_mapping.iter() {
                    variables_to_bind.push((glsl_name, mapping.texture));
                    if let Some(handle) = mapping.sampler {
                        variables_to_bind.push((glsl_name, handle));
                    }
                }
                for (&handle, glsl_name) in reflection.uniforms.iter() {
                    variables_to_bind.push((glsl_name, handle));
                }

                for (glsl_name, var_handle) in variables_to_bind.drain(..) {
                    let var = &sf.shader.module.global_variables[var_handle];
                    let var_name = var.name.as_ref().unwrap().as_str();
                    let (group_index, binding_index) = group_layouts
                        .iter()
                        .enumerate()
                        .find_map(|(group_index, layout)| {
                            layout
                                .bindings
                                .iter()
                                .position(|&(name, _)| name == var_name)
                                .map(|binding_index| (group_index, binding_index))
                        })
                        .unwrap_or_else(|| {
                            panic!("Shader variable {} is not found in the bindings", var_name)
                        });

                    let targets = &mut group_mappings[group_index].targets[binding_index];
                    match group_layouts[group_index].bindings[binding_index].1 {
                        crate::ShaderBinding::Texture | crate::ShaderBinding::Sampler => {
                            if let Some(ref location) = gl.get_uniform_location(program, glsl_name)
                            {
                                let mut slots = [0i32];
                                gl.get_uniform_i32(program, location, &mut slots);
                                targets.push(slots[0] as u32);
                            }
                        }
                        crate::ShaderBinding::Buffer => {
                            if let Some(index) =
                                gl.get_shader_storage_block_index(program, glsl_name)
                            {
                                let params = gl.get_program_resource_i32(
                                    program,
                                    glow::SHADER_STORAGE_BLOCK,
                                    index,
                                    &[glow::BUFFER_BINDING],
                                );
                                targets.push(params[0] as u32);
                            }
                        }
                        crate::ShaderBinding::TextureArray { .. }
                        | crate::ShaderBinding::BufferArray { .. }
                        | crate::ShaderBinding::AccelerationStructure => {
                            unimplemented!()
                        }
                        crate::ShaderBinding::Plain { size } => {
                            if let Some(index) = gl.get_uniform_block_index(program, glsl_name) {
                                let expected_size = gl.get_active_uniform_block_parameter_i32(
                                    program,
                                    index,
                                    glow::UNIFORM_BLOCK_DATA_SIZE,
                                ) as u32;
                                let rounded_up_size = super::round_up_uniform_size(size);
                                assert!(
                                    expected_size <= rounded_up_size,
                                    "Shader expects block[{}] size {}, but data has size of {} (rounded up to {})",
                                    index,
                                    expected_size,
                                    size,
                                    rounded_up_size,
                                );
                                let slot = if force_uniform_block_assignment {
                                    gl.uniform_block_binding(program, index, index);
                                    index
                                } else {
                                    gl.get_active_uniform_block_parameter_i32(
                                        program,
                                        index,
                                        glow::UNIFORM_BLOCK_BINDING,
                                    ) as u32
                                };
                                targets.push(slot);
                            }
                        }
                    }
                }
            }

            for (shader, _) in baked_shaders {
                gl.delete_shader(shader);
            }
        }
        gl.use_program(None);

        super::PipelineInner {
            program,
            group_mappings,
            vertex_attribute_infos: attributes.into_boxed_slice(),
            color_targets: Box::new([]),
        }
    }

    unsafe fn destroy_pipeline(&self, inner: &mut super::PipelineInner) {
        let gl = self.lock();
        gl.delete_program(inner.program);
    }
}

#[hidden_trait::expose]
impl crate::traits::ShaderDevice for super::Context {
    type ComputePipeline = super::ComputePipeline;
    type RenderPipeline = super::RenderPipeline;

    fn create_compute_pipeline(&self, desc: crate::ComputePipelineDesc) -> super::ComputePipeline {
        let wg_size = desc.compute.shader.module.entry_points[desc.compute.entry_point_index()]
            .workgroup_size;
        let inner = unsafe {
            self.create_pipeline(
                &[desc.compute],
                desc.data_layouts,
                &[],
                desc.name,
                glsl::WriterFlags::empty(),
            )
        };
        super::ComputePipeline { inner, wg_size }
    }

    fn destroy_compute_pipeline(&self, pipeline: &mut super::ComputePipeline) {
        unsafe {
            self.destroy_pipeline(&mut pipeline.inner);
        }
    }

    fn create_render_pipeline(&self, desc: crate::RenderPipelineDesc) -> super::RenderPipeline {
        let extra_flags = if desc.primitive.topology == crate::PrimitiveTopology::PointList {
            glsl::WriterFlags::FORCE_POINT_SIZE
        } else {
            glsl::WriterFlags::empty()
        };

        let shaders = match desc.fragment {
            Some(fs) => vec![desc.vertex, fs],
            None => vec![desc.vertex],
        };

        let mut inner = unsafe {
            self.create_pipeline(
                &shaders,
                desc.data_layouts,
                desc.vertex_fetches,
                desc.name,
                extra_flags,
            )
        };

        inner.color_targets = conflate(desc.color_targets.iter().map(|t| (t.blend, t.write_mask)));

        if !self
            .capabilities
            .contains(super::Capabilities::DRAW_BUFFERS_INDEXED)
        {
            assert!(
                inner.color_targets.len() <= 1,
                "separate blend states or write masks of multiple color targets are not supported"
            );
        }

        super::RenderPipeline {
            inner,
            topology: desc.primitive.topology,
        }
    }

    fn destroy_render_pipeline(&self, pipeline: &mut super::RenderPipeline) {
        unsafe {
            self.destroy_pipeline(&mut pipeline.inner);
        }
    }
}
