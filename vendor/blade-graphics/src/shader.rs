use once_cell::sync::Lazy;
use std::borrow::Cow;

impl From<naga::ShaderStage> for super::ShaderVisibility {
    fn from(stage: naga::ShaderStage) -> Self {
        match stage {
            naga::ShaderStage::Compute => Self::COMPUTE,
            naga::ShaderStage::Vertex => Self::VERTEX,
            naga::ShaderStage::Fragment => Self::FRAGMENT,
            _ => Self::empty(),
        }
    }
}

impl super::Context {
    pub fn try_create_shader(
        &self,
        desc: super::ShaderDesc,
    ) -> Result<super::Shader, &'static str> {
        let module = naga::front::wgsl::parse_str(desc.source).map_err(|e| {
            e.emit_to_stderr_with_path(desc.source, "");
            "compilation failed"
        })?;

        let device_caps = self.capabilities();

        // Bindings are set up at pipeline creation, ignore here
        let flags = naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::BINDINGS;
        let mut caps = naga::valid::Capabilities::empty();
        caps.set(
            naga::valid::Capabilities::RAY_QUERY | naga::valid::Capabilities::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
            !device_caps.ray_query.is_empty(),
        );
        let info = naga::valid::Validator::new(flags, caps)
            .validate(&module)
            .map_err(|e| {
                crate::util::emit_annotated_error(&e, "", desc.source);
                crate::util::print_err(&e);
                "validation failed"
            })?;

        Ok(super::Shader {
            module,
            info,
            source: desc.source.to_owned(),
        })
    }

    pub fn create_shader(&self, desc: super::ShaderDesc) -> super::Shader {
        self.try_create_shader(desc).unwrap()
    }
}

pub static EMPTY_CONSTANTS: Lazy<super::PipelineConstants> = Lazy::new(Default::default);

impl super::Shader {
    pub fn at<'a>(&'a self, entry_point: &'a str) -> super::ShaderFunction<'a> {
        super::ShaderFunction {
            shader: self,
            entry_point,
            constants: Lazy::force(&EMPTY_CONSTANTS),
        }
    }

    pub fn with_constants<'a>(
        &'a self,
        entry_point: &'a str,
        constants: &'a super::PipelineConstants,
    ) -> super::ShaderFunction<'a> {
        super::ShaderFunction {
            shader: self,
            entry_point,
            constants,
        }
    }

    pub fn resolve_constants<'a>(
        &'a self,
        constants: &super::PipelineConstants,
    ) -> (naga::Module, Cow<'a, naga::valid::ModuleInfo>) {
        let (module, info) =
            naga::back::pipeline_constants::process_overrides(&self.module, &self.info, constants)
                .unwrap();
        (module.into_owned(), info)
    }

    pub fn get_struct_size(&self, struct_name: &str) -> u32 {
        match self
            .module
            .types
            .iter()
            .find(|&(_, ty)| ty.name.as_deref() == Some(struct_name))
        {
            Some((_, ty)) => match ty.inner {
                naga::TypeInner::Struct { members: _, span } => span,
                _ => panic!("Type '{struct_name}' is not a struct in the shader"),
            },
            None => panic!("Struct '{struct_name}' is not found in the shader"),
        }
    }

    pub fn check_struct_size<T>(&self) {
        use std::{any::type_name, mem::size_of};
        let name = type_name::<T>().rsplit("::").next().unwrap();
        assert_eq!(
            size_of::<T>(),
            self.get_struct_size(name) as usize,
            "Host struct '{name}' size doesn't match the shader"
        );
    }

    pub(crate) fn fill_resource_bindings(
        module: &mut naga::Module,
        sd_infos: &mut [crate::ShaderDataInfo],
        naga_stage: naga::ShaderStage,
        ep_info: &naga::valid::FunctionInfo,
        group_layouts: &[&crate::ShaderDataLayout],
    ) {
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).unwrap();

        for (handle, var) in module.global_variables.iter_mut() {
            if ep_info[handle].is_empty() {
                continue;
            }
            let var_access = match var.space {
                naga::AddressSpace::Storage { access } => access,
                naga::AddressSpace::Uniform | naga::AddressSpace::Handle => {
                    naga::StorageAccess::empty()
                }
                _ => continue,
            };

            let expected_binding = |proto_binding: crate::ShaderBinding| {
                match module.types[var.ty].inner {
                    naga::TypeInner::Image {
                        class: naga::ImageClass::Storage { access, format: _ },
                        ..
                    } => (crate::ShaderBinding::Texture, access),
                    naga::TypeInner::Image { .. } => {
                        (crate::ShaderBinding::Texture, naga::StorageAccess::empty())
                    }
                    naga::TypeInner::Sampler { .. } => {
                        (crate::ShaderBinding::Sampler, naga::StorageAccess::empty())
                    }
                    naga::TypeInner::AccelerationStructure { vertex_return: _ } => (
                        crate::ShaderBinding::AccelerationStructure,
                        naga::StorageAccess::empty(),
                    ),
                    naga::TypeInner::BindingArray { base, size: _ } => {
                        let count = match proto_binding {
                            crate::ShaderBinding::TextureArray { count } => count,
                            crate::ShaderBinding::BufferArray { count } => count,
                            _ => 0,
                        };
                        let proto = match module.types[base].inner {
                            naga::TypeInner::Image { .. } => {
                                crate::ShaderBinding::TextureArray { count }
                            }
                            naga::TypeInner::Struct { .. } => {
                                crate::ShaderBinding::BufferArray { count }
                            }
                            ref other => panic!("Unsupported binding array for {:?}", other),
                        };
                        (proto, var_access)
                    }
                    _ => {
                        let type_layout = &layouter[var.ty];
                        let proto = if var_access.is_empty()
                            && proto_binding != crate::ShaderBinding::Buffer
                        {
                            crate::ShaderBinding::Plain {
                                size: type_layout.size,
                            }
                        } else {
                            crate::ShaderBinding::Buffer
                        };
                        (proto, var_access)
                    }
                }
            };

            if let Some(binding) = var.binding {
                let group_index = binding.group as usize;
                let binding_index = binding.binding as usize;
                let (layout, info) = group_layouts
                    .iter()
                    .zip(sd_infos.iter_mut())
                    .nth(group_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "Explicit binding uses unknown bind group {} for '{}'",
                            binding.group,
                            var.name.as_deref().unwrap_or("?")
                        )
                    });
                let (_, proto_binding) = layout
                    .bindings
                    .get(binding_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "Explicit binding uses unknown binding slot {} in group {} for '{}'",
                            binding.binding,
                            binding.group,
                            var.name.as_deref().unwrap_or("?")
                        )
                    });
                let (expected_proto, access) = expected_binding(*proto_binding);
                assert_eq!(
                    *proto_binding, expected_proto,
                    "Mismatched type for binding '{}' at group {} binding {}",
                    var.name.as_deref().unwrap_or("?"),
                    binding.group,
                    binding.binding
                );
                info.visibility |= naga_stage.into();
                info.binding_access[binding_index] |= access;
            } else {
                let var_name = var.name.as_ref().unwrap();
                for (group_index, (&layout, info)) in
                    group_layouts.iter().zip(sd_infos.iter_mut()).enumerate()
                {
                    if let Some((binding_index, &(_, proto_binding))) = layout
                        .bindings
                        .iter()
                        .enumerate()
                        .find(|&(_, &(name, _))| name == var_name)
                    {
                        let (expected_proto, access) = expected_binding(proto_binding);
                        assert_eq!(
                            proto_binding, expected_proto,
                            "Mismatched type for binding '{}'",
                            var_name
                        );
                        var.binding = Some(naga::ResourceBinding {
                            group: group_index as u32,
                            binding: binding_index as u32,
                        });
                        info.visibility |= naga_stage.into();
                        info.binding_access[binding_index] |= access;
                        break;
                    }
                }
            }

            assert!(
                var.binding.is_some(),
                "Unable to resolve binding for '{}' in stage '{:?}'",
                var.name.as_deref().unwrap_or("?"),
                naga_stage,
            );
        }
    }

    pub(crate) fn fill_vertex_locations(
        module: &mut naga::Module,
        selected_ep_index: usize,
        fetch_states: &[crate::VertexFetchState],
    ) -> Vec<crate::VertexAttributeMapping> {
        let mut attribute_mappings = Vec::new();
        for (ep_index, ep) in module.entry_points.iter().enumerate() {
            let mut location = 0;
            if ep.stage != naga::ShaderStage::Vertex {
                continue;
            }

            for argument in ep.function.arguments.iter() {
                if argument.binding.is_some() {
                    continue;
                }

                let arg_name = match argument.name {
                    Some(ref name) => name.as_str(),
                    None => "?",
                };
                let mut ty = module.types[argument.ty].clone();
                let members = match ty.inner {
                    naga::TypeInner::Struct {
                        ref mut members, ..
                    } => members,
                    ref other => {
                        log::error!("Unexpected type for '{}': {:?}", arg_name, other);
                        continue;
                    }
                };

                if ep_index == selected_ep_index {
                    log::debug!("Processing vertex argument: {}", arg_name);

                    let mut next_location = 0u32;
                    let mut mappings: Vec<Option<crate::VertexAttributeMapping>> = Vec::new();
                    let mut changed = false;
                    for member in members.iter_mut() {
                        let member_name = match member.name {
                            Some(ref name) => name.as_str(),
                            None => "?",
                        };
                        match member.binding {
                            Some(naga::Binding::BuiltIn(_)) => {
                                continue;
                            }
                            Some(naga::Binding::Location { .. }) | None => {}
                        }
                        let explicit_location = match member.binding {
                            Some(naga::Binding::Location { location, .. }) => Some(location),
                            _ => None,
                        };
                        let location = explicit_location.unwrap_or_else(|| {
                            let loc = next_location;
                            next_location = next_location.saturating_add(1);
                            loc
                        });
                        if explicit_location.is_some() {
                            next_location = next_location.max(location.saturating_add(1));
                        }
                        if member.binding.is_none() {
                            member.binding = Some(naga::Binding::Location {
                                location,
                                interpolation: None,
                                sampling: None,
                                blend_src: None,
                            });
                            changed = true;
                        }
                        let mut mapping_found = false;
                        for (buffer_index, vertex_fetch) in fetch_states.iter().enumerate() {
                            for (attribute_index, &(_, attr)) in
                                vertex_fetch.layout.attributes.iter().enumerate()
                            {
                                if let Some(attr_location) = attr.location {
                                    if attr_location == location {
                                        let idx = location as usize;
                                        while mappings.len() <= idx {
                                            mappings.push(None);
                                        }
                                        mappings[idx] = Some(crate::VertexAttributeMapping {
                                            location,
                                            buffer_index,
                                            attribute_index,
                                        });
                                        mapping_found = true;
                                        break;
                                    }
                                }
                            }
                            if mapping_found {
                                break;
                            }
                        }
                        if !mapping_found {
                            for (buffer_index, vertex_fetch) in fetch_states.iter().enumerate() {
                                for (attribute_index, &(at_name, _)) in
                                    vertex_fetch.layout.attributes.iter().enumerate()
                                {
                                    if at_name == member_name {
                                        let idx = location as usize;
                                        while mappings.len() <= idx {
                                            mappings.push(None);
                                        }
                                        mappings[idx] = Some(crate::VertexAttributeMapping {
                                            location,
                                            buffer_index,
                                            attribute_index,
                                        });
                                        mapping_found = true;
                                        break;
                                    }
                                }
                                if mapping_found {
                                    break;
                                }
                            }
                        }
                        assert!(
                            mapping_found,
                            "Field {} is not covered by the vertex fetch layouts!",
                            member_name
                        );
                    }
                    for (index, mapping) in mappings.into_iter().enumerate() {
                        let mapping = mapping.unwrap_or_else(|| {
                            panic!(
                                "Missing vertex attribute mapping for location {}",
                                index
                            )
                        });
                        attribute_mappings.push(mapping);
                    }
                    if changed {
                        module.types.replace(argument.ty, ty);
                    }
                } else {
                    // Just fill out the locations for the module to be valid
                    let mut changed = false;
                    for member in members.iter_mut() {
                        match member.binding {
                            Some(naga::Binding::Location { location: loc, .. }) => {
                                location = location.max(loc.saturating_add(1));
                            }
                            Some(naga::Binding::BuiltIn(_)) => {}
                            _ => {
                                member.binding = Some(naga::Binding::Location {
                                    location,
                                    interpolation: None,
                                    sampling: None,
                                    blend_src: None,
                                });
                                location += 1;
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        module.types.replace(argument.ty, ty);
                    }
                }
            }
        }
        attribute_mappings
    }
}
