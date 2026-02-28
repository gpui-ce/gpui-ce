use ash::vk;
use std::{str, time::Duration};

impl super::CrashHandler {
    fn add_marker(&mut self, marker: &str) -> u32 {
        if self.next_offset < self.raw_string.len() {
            self.raw_string[self.next_offset] = b'|';
            self.next_offset += 1;
        }
        let len = marker.as_bytes().len().min(self.raw_string.len());
        if self.next_offset + len > self.raw_string.len() {
            self.next_offset = 0;
        }
        let start = self.next_offset;
        self.next_offset += len;
        let end = self.next_offset;
        self.raw_string[start..end].copy_from_slice(&marker.as_bytes()[..len]);
        start as u32 | (end << 16) as u32
    }

    pub(super) fn extract(&self, id: u32) -> (&str, &str) {
        let start = id as usize & 0xFFFF;
        let end = (id >> 16) as usize;
        let history = str::from_utf8(&self.raw_string[..start]).unwrap_or_default();
        let marker = str::from_utf8(&self.raw_string[start..end]).unwrap();
        (history, marker)
    }
}

impl super::PipelineContext<'_> {
    #[inline]
    fn write<T>(&mut self, index: u32, value: T) {
        let offset = self.template_offsets[index as usize];
        unsafe {
            std::ptr::write(
                self.update_data.as_mut_ptr().offset(offset as isize) as *mut T,
                value,
            )
        };
    }

    #[inline]
    fn write_array<I: Iterator>(&mut self, index: u32, iter: I) {
        let base_offset = self.template_offsets[index as usize];
        let base_ptr =
            unsafe { self.update_data.as_mut_ptr().offset(base_offset as isize) as *mut I::Item };
        for (i, value) in iter.enumerate() {
            unsafe { std::ptr::write(base_ptr.add(i), value) };
        }
    }
}

impl<T: bytemuck::Pod> crate::ShaderBindable for T {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        ctx.write(index, *self);
    }
}
impl crate::ShaderBindable for super::TextureView {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        ctx.write(
            index,
            vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: self.raw,
                image_layout: vk::ImageLayout::GENERAL,
            },
        );
    }
}
impl<'a, const N: crate::ResourceIndex> crate::ShaderBindable for &'a crate::TextureArray<N> {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        ctx.write_array(
            index,
            self.data.iter().map(|view| vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: view.raw,
                image_layout: vk::ImageLayout::GENERAL,
            }),
        );
    }
}
impl crate::ShaderBindable for super::Sampler {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        ctx.write(
            index,
            vk::DescriptorImageInfo {
                sampler: self.raw,
                image_view: vk::ImageView::null(),
                image_layout: vk::ImageLayout::UNDEFINED,
            },
        );
    }
}
impl crate::ShaderBindable for crate::BufferPiece {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        ctx.write(
            index,
            vk::DescriptorBufferInfo {
                buffer: self.buffer.raw,
                offset: self.offset,
                range: vk::WHOLE_SIZE,
            },
        );
    }
}
impl<'a, const N: crate::ResourceIndex> crate::ShaderBindable for &'a crate::BufferArray<N> {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        ctx.write_array(
            index,
            self.data.iter().map(|piece| vk::DescriptorBufferInfo {
                buffer: piece.buffer.raw,
                offset: piece.offset,
                range: vk::WHOLE_SIZE,
            }),
        );
    }
}
impl crate::ShaderBindable for super::AccelerationStructure {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        ctx.write(index, self.raw);
    }
}

impl crate::TexturePiece {
    fn subresource_layers(&self) -> vk::ImageSubresourceLayers {
        vk::ImageSubresourceLayers {
            aspect_mask: super::map_aspects(self.texture.format.aspects()),
            mip_level: self.mip_level,
            base_array_layer: self.array_layer,
            layer_count: 1,
        }
    }
}

fn map_origin(origin: &[u32; 3]) -> vk::Offset3D {
    vk::Offset3D {
        x: origin[0] as i32,
        y: origin[1] as i32,
        z: origin[2] as i32,
    }
}

fn make_buffer_image_copy(
    buffer: &crate::BufferPiece,
    bytes_per_row: u32,
    texture: &crate::TexturePiece,
    size: &crate::Extent,
) -> vk::BufferImageCopy {
    let block_info = texture.texture.format.block_info();
    vk::BufferImageCopy {
        buffer_offset: buffer.offset,
        buffer_row_length: block_info.dimensions.0 as u32
            * (bytes_per_row / block_info.size as u32),
        buffer_image_height: 0,
        image_subresource: texture.subresource_layers(),
        image_offset: map_origin(&texture.origin),
        image_extent: super::map_extent_3d(size),
    }
}

fn map_render_target(rt: &crate::RenderTarget) -> vk::RenderingAttachmentInfo<'static> {
    let mut vk_info = vk::RenderingAttachmentInfo::default()
        .image_view(rt.view.raw)
        .image_layout(vk::ImageLayout::GENERAL);

    match rt.init_op {
        crate::InitOp::Load => vk_info = vk_info.load_op(vk::AttachmentLoadOp::LOAD),
        crate::InitOp::DontCare => vk_info = vk_info.load_op(vk::AttachmentLoadOp::DONT_CARE),

        crate::InitOp::Clear(color) => {
            let cv = if rt.view.aspects.contains(crate::TexelAspects::COLOR) {
                vk::ClearValue {
                    color: match color {
                        crate::TextureColor::TransparentBlack => {
                            vk::ClearColorValue { float32: [0.0; 4] }
                        }
                        crate::TextureColor::OpaqueBlack => vk::ClearColorValue {
                            float32: [0.0, 0.0, 0.0, 1.0],
                        },
                        crate::TextureColor::White => vk::ClearColorValue { float32: [1.0; 4] },
                    },
                }
            } else {
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: color.depth_clear_value(),
                        stencil: color.stencil_clear_value(),
                    },
                }
            };

            vk_info = vk_info.load_op(vk::AttachmentLoadOp::CLEAR).clear_value(cv);
        }
    }

    if let crate::FinishOp::ResolveTo(resolve_view) = rt.finish_op {
        vk_info = vk_info
            .resolve_image_view(resolve_view.raw)
            .resolve_image_layout(vk::ImageLayout::GENERAL)
            .resolve_mode(vk::ResolveModeFlags::AVERAGE);
    }

    vk_info.store_op = match rt.finish_op {
        crate::FinishOp::Store => vk::AttachmentStoreOp::STORE,
        crate::FinishOp::Discard => vk::AttachmentStoreOp::DONT_CARE,
        crate::FinishOp::Ignore => vk::AttachmentStoreOp::DONT_CARE,
        crate::FinishOp::ResolveTo(..) => {
            /*
                TODO: DONT_CARE is most optimal in many cases where the msaa texture itself is never read afterwards but only the resolved,
                      but how can the user specify this in blade?
                      https://docs.vulkan.org/samples/latest/samples/performance/msaa/README.html#_best_practice_summary
            */

            // vk::AttachmentStoreOp::STORE
            vk::AttachmentStoreOp::DONT_CARE
        }
    };

    vk_info
}

fn end_pass(device: &super::Device, cmd_buf: vk::CommandBuffer) {
    if device.command_scope.is_some() {
        unsafe {
            device.debug_utils.cmd_end_debug_utils_label(cmd_buf);
        }
    }
}

impl super::CommandEncoder {
    fn add_marker(&mut self, marker: &str) {
        if let Some(ref mut ch) = self.crash_handler {
            let id = ch.add_marker(marker);
            unsafe {
                self.device
                    .buffer_marker
                    .as_ref()
                    .unwrap()
                    .cmd_write_buffer_marker(
                        self.buffers[0].raw,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        ch.marker_buf.raw,
                        0,
                        id,
                    );
            }
        }
    }

    fn add_timestamp(&mut self, label: &str) {
        if let Some(_) = self.device.timing {
            let cmd_buf = self.buffers.first_mut().unwrap();
            if cmd_buf.timed_pass_names.len() == crate::limits::PASS_COUNT {
                log::warn!("Reached the maximum for `limits::PASS_COUNT`, skipping the timer");
                return;
            }
            let index = cmd_buf.timed_pass_names.len() as u32;
            unsafe {
                self.device.core.cmd_write_timestamp(
                    cmd_buf.raw,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    cmd_buf.query_pool,
                    index,
                );
            }
            cmd_buf.timed_pass_names.push(label.to_string());
        }
    }

    fn begin_pass(&mut self, label: &str) {
        self.barrier();
        self.add_marker(label);
        self.add_timestamp(label);

        if let Some(_) = self.device.command_scope {
            self.temp_label.clear();
            self.temp_label.extend_from_slice(label.as_bytes());
            self.temp_label.push(0);
            unsafe {
                self.device.debug_utils.cmd_begin_debug_utils_label(
                    self.buffers[0].raw,
                    &vk::DebugUtilsLabelEXT {
                        p_label_name: self.temp_label.as_ptr() as *const _,
                        ..Default::default()
                    },
                )
            }
        }
    }

    pub(super) fn finish(&mut self) -> vk::CommandBuffer {
        self.barrier();
        self.add_marker("finish");
        let cmd_buf = self.buffers.first_mut().unwrap();
        unsafe {
            if self.device.timing.is_some() {
                let index = cmd_buf.timed_pass_names.len() as u32;
                self.device.core.cmd_write_timestamp(
                    cmd_buf.raw,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    cmd_buf.query_pool,
                    index,
                );
            }
            self.device.core.end_command_buffer(cmd_buf.raw).unwrap();
        }
        cmd_buf.raw
    }

    fn barrier(&mut self) {
        let wa = &self.device.workarounds;
        let barrier = vk::MemoryBarrier {
            src_access_mask: vk::AccessFlags::MEMORY_WRITE | wa.extra_sync_src_access,
            dst_access_mask: vk::AccessFlags::MEMORY_READ
                | vk::AccessFlags::MEMORY_WRITE
                | wa.extra_sync_dst_access,
            ..Default::default()
        };
        unsafe {
            self.device.core.cmd_pipeline_barrier(
                self.buffers[0].raw,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[barrier],
                &[],
                &[],
            );
        }
    }

    pub fn transfer(&mut self, label: &str) -> super::TransferCommandEncoder {
        self.begin_pass(label);
        super::TransferCommandEncoder {
            raw: self.buffers[0].raw,
            device: &self.device,
        }
    }

    pub fn acceleration_structure(
        &mut self,
        label: &str,
    ) -> super::AccelerationStructureCommandEncoder {
        self.begin_pass(label);
        super::AccelerationStructureCommandEncoder {
            raw: self.buffers[0].raw,
            device: &self.device,
        }
    }

    pub fn compute(&mut self, label: &str) -> super::ComputeCommandEncoder {
        self.begin_pass(label);
        super::ComputeCommandEncoder {
            cmd_buf: self.buffers.first_mut().unwrap(),
            device: &self.device,
            update_data: &mut self.update_data,
        }
    }

    pub fn render(
        &mut self,
        label: &str,
        targets: crate::RenderTargetSet,
    ) -> super::RenderCommandEncoder {
        self.begin_pass(label);

        let mut target_size = [0u16; 2];
        let mut color_attachments = Vec::with_capacity(targets.colors.len());
        let depth_stencil_attachment;
        for rt in targets.colors {
            target_size = rt.view.target_size;
            color_attachments.push(map_render_target(rt));
        }

        let mut rendering_info = vk::RenderingInfoKHR::default()
            .layer_count(1)
            .color_attachments(&color_attachments);

        if let Some(rt) = targets.depth_stencil {
            target_size = rt.view.target_size;
            depth_stencil_attachment = map_render_target(&rt);
            if rt.view.aspects.contains(crate::TexelAspects::DEPTH) {
                rendering_info = rendering_info.depth_attachment(&depth_stencil_attachment);
            }
            if rt.view.aspects.contains(crate::TexelAspects::STENCIL) {
                rendering_info = rendering_info.stencil_attachment(&depth_stencil_attachment);
            }
        }

        let render_area = crate::ScissorRect {
            x: 0,
            y: 0,
            w: target_size[0] as u32,
            h: target_size[1] as u32,
        }
        .to_vk();
        let viewport = crate::Viewport {
            x: 0.0,
            y: 0.0,
            w: target_size[0] as f32,
            h: target_size[1] as f32,
            depth: 0.0..1.0,
        }
        .to_vk();
        rendering_info.render_area = render_area;

        let cmd_buf = self.buffers.first_mut().unwrap();
        unsafe {
            self.device
                .core
                .cmd_set_viewport(cmd_buf.raw, 0, &[viewport]);
            self.device
                .core
                .cmd_set_scissor(cmd_buf.raw, 0, &[render_area]);
            self.device
                .dynamic_rendering
                .cmd_begin_rendering(cmd_buf.raw, &rendering_info);
        };

        super::RenderCommandEncoder {
            cmd_buf,
            device: &self.device,
            update_data: &mut self.update_data,
        }
    }

    pub(super) fn check_gpu_crash<T>(&self, ret: Result<T, vk::Result>) -> Option<T> {
        match ret {
            Ok(value) => Some(value),
            Err(vk::Result::ERROR_DEVICE_LOST) => match self.crash_handler {
                Some(ref ch) => {
                    let last_id = unsafe { *(ch.marker_buf.data() as *mut u32) };
                    if last_id != 0 {
                        let (history, last_marker) = ch.extract(last_id);
                        log::error!("Last GPU executed marker is '{last_marker}'");
                        log::info!("Marker history: {}", history);
                    }
                    panic!("GPU has crashed in {}", ch.name);
                }
                None => {
                    panic!("GPU has crashed, and no debug information is available.");
                }
            },
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                log::warn!("GPU frame is out of date");
                None
            }
            Err(other) => panic!("GPU error {}", other),
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::CommandEncoder for super::CommandEncoder {
    type Texture = super::Texture;
    type Frame = super::Frame;

    fn start(&mut self) {
        self.buffers.rotate_left(1);
        let cmd_buf = self.buffers.first_mut().unwrap();
        self.device
            .reset_descriptor_pool(&mut cmd_buf.descriptor_pool);

        let vk_info = vk::CommandBufferBeginInfo {
            flags: vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT,
            ..Default::default()
        };
        unsafe {
            self.device
                .core
                .begin_command_buffer(cmd_buf.raw, &vk_info)
                .unwrap();
        }

        if let Some(ref timing) = self.device.timing {
            self.timings.clear();
            if !cmd_buf.timed_pass_names.is_empty() {
                let mut timestamps = [0u64; super::QUERY_POOL_SIZE];
                unsafe {
                    self.device
                        .core
                        .get_query_pool_results(
                            cmd_buf.query_pool,
                            0,
                            &mut timestamps[..cmd_buf.timed_pass_names.len() + 1],
                            vk::QueryResultFlags::TYPE_64,
                        )
                        .unwrap();
                }
                let mut prev = timestamps[0];
                for (name, &ts) in cmd_buf
                    .timed_pass_names
                    .drain(..)
                    .zip(timestamps[1..].iter())
                {
                    let diff = (ts - prev) as f32 * timing.period;
                    prev = ts;
                    self.timings.push((name, Duration::from_nanos(diff as _)));
                }
            }
            unsafe {
                self.device.core.cmd_reset_query_pool(
                    cmd_buf.raw,
                    cmd_buf.query_pool,
                    0,
                    super::QUERY_POOL_SIZE as u32,
                );
            }
        }
    }

    fn init_texture(&mut self, texture: super::Texture) {
        let barrier = vk::ImageMemoryBarrier {
            old_layout: vk::ImageLayout::UNDEFINED,
            new_layout: vk::ImageLayout::GENERAL,
            image: texture.raw,
            subresource_range: vk::ImageSubresourceRange {
                aspect_mask: super::map_aspects(texture.format.aspects()),
                base_mip_level: 0,
                level_count: vk::REMAINING_MIP_LEVELS,
                base_array_layer: 0,
                layer_count: vk::REMAINING_ARRAY_LAYERS,
            },
            ..Default::default()
        };
        unsafe {
            self.device.core.cmd_pipeline_barrier(
                self.buffers[0].raw,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    fn present(&mut self, frame: super::Frame) {
        let image_index = match frame.image_index {
            Some(index) => index,
            None => {
                //Note: image was out of date, we can't present it
                return;
            }
        };

        assert_eq!(self.present, None);
        let wa = &self.device.workarounds;
        self.present = Some(super::Presentation {
            swapchain: frame.swapchain.raw,
            image_index,
            acquire_semaphore: frame.internal.acquire_semaphore,
            present_semaphore: frame.internal.present_semaphore,
        });

        let barrier = vk::ImageMemoryBarrier {
            old_layout: vk::ImageLayout::GENERAL,
            new_layout: vk::ImageLayout::PRESENT_SRC_KHR,
            image: frame.internal.image,
            subresource_range: vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            },
            src_access_mask: vk::AccessFlags::MEMORY_WRITE | wa.extra_sync_src_access,
            ..Default::default()
        };
        unsafe {
            self.device.core.cmd_pipeline_barrier(
                self.buffers[0].raw,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    fn timings(&self) -> &crate::Timings {
        &self.timings
    }
}

#[hidden_trait::expose]
impl crate::traits::TransferEncoder for super::TransferCommandEncoder<'_> {
    type BufferPiece = crate::BufferPiece;
    type TexturePiece = crate::TexturePiece;

    fn fill_buffer(&mut self, dst: crate::BufferPiece, size: u64, value: u8) {
        let value_u32 = (value as u32) * 0x1010101;
        unsafe {
            self.device
                .core
                .cmd_fill_buffer(self.raw, dst.buffer.raw, dst.offset, size, value_u32)
        };
    }

    fn copy_buffer_to_buffer(
        &mut self,
        src: crate::BufferPiece,
        dst: crate::BufferPiece,
        size: u64,
    ) {
        let copy = vk::BufferCopy {
            src_offset: src.offset,
            dst_offset: dst.offset,
            size,
        };
        unsafe {
            self.device
                .core
                .cmd_copy_buffer(self.raw, src.buffer.raw, dst.buffer.raw, &[copy])
        };
    }

    fn copy_texture_to_texture(
        &mut self,
        src: crate::TexturePiece,
        dst: crate::TexturePiece,
        size: crate::Extent,
    ) {
        let copy = vk::ImageCopy {
            src_subresource: src.subresource_layers(),
            src_offset: map_origin(&src.origin),
            dst_subresource: dst.subresource_layers(),
            dst_offset: map_origin(&dst.origin),
            extent: super::map_extent_3d(&size),
        };
        unsafe {
            self.device.core.cmd_copy_image(
                self.raw,
                src.texture.raw,
                vk::ImageLayout::GENERAL,
                dst.texture.raw,
                vk::ImageLayout::GENERAL,
                &[copy],
            )
        };
    }

    fn copy_buffer_to_texture(
        &mut self,
        src: crate::BufferPiece,
        bytes_per_row: u32,
        dst: crate::TexturePiece,
        size: crate::Extent,
    ) {
        let copy = make_buffer_image_copy(&src, bytes_per_row, &dst, &size);
        unsafe {
            self.device.core.cmd_copy_buffer_to_image(
                self.raw,
                src.buffer.raw,
                dst.texture.raw,
                vk::ImageLayout::GENERAL,
                &[copy],
            )
        };
    }

    fn copy_texture_to_buffer(
        &mut self,
        src: crate::TexturePiece,
        dst: crate::BufferPiece,
        bytes_per_row: u32,
        size: crate::Extent,
    ) {
        let copy = make_buffer_image_copy(&dst, bytes_per_row, &src, &size);
        unsafe {
            self.device.core.cmd_copy_image_to_buffer(
                self.raw,
                src.texture.raw,
                vk::ImageLayout::GENERAL,
                dst.buffer.raw,
                &[copy],
            )
        };
    }
}

impl Drop for super::TransferCommandEncoder<'_> {
    fn drop(&mut self) {
        end_pass(self.device, self.raw);
    }
}

#[hidden_trait::expose]
impl crate::traits::AccelerationStructureEncoder
    for super::AccelerationStructureCommandEncoder<'_>
{
    type AccelerationStructure = crate::AccelerationStructure;
    type AccelerationStructureMesh = crate::AccelerationStructureMesh;
    type BufferPiece = crate::BufferPiece;

    fn build_bottom_level(
        &mut self,
        acceleration_structure: super::AccelerationStructure,
        meshes: &[crate::AccelerationStructureMesh],
        scratch_data: crate::BufferPiece,
    ) {
        let rt = self.device.ray_tracing.as_ref().unwrap();
        let mut blas_input = self.device.map_acceleration_structure_meshes(meshes);
        blas_input.build_info.dst_acceleration_structure = acceleration_structure.raw;
        let scratch_address = self.device.get_device_address(&scratch_data);
        assert_eq!(
            scratch_address & rt.scratch_buffer_alignment,
            0,
            "BLAS scratch address {scratch_address} is not aligned"
        );
        blas_input.build_info.scratch_data = vk::DeviceOrHostAddressKHR {
            device_address: scratch_address,
        };

        unsafe {
            rt.acceleration_structure.cmd_build_acceleration_structures(
                self.raw,
                &[blas_input.build_info],
                &[&blas_input.build_range_infos],
            );
        }
    }

    fn build_top_level(
        &mut self,
        acceleration_structure: super::AccelerationStructure,
        _bottom_level: &[super::AccelerationStructure],
        instance_count: u32,
        instance_data: crate::BufferPiece,
        scratch_data: crate::BufferPiece,
    ) {
        let build_range_info = vk::AccelerationStructureBuildRangeInfoKHR {
            primitive_count: instance_count,
            primitive_offset: 0,
            first_vertex: 0,
            transform_offset: 0,
        };
        let geometry = vk::AccelerationStructureGeometryKHR {
            geometry_type: vk::GeometryTypeKHR::INSTANCES,
            geometry: vk::AccelerationStructureGeometryDataKHR {
                instances: vk::AccelerationStructureGeometryInstancesDataKHR {
                    data: vk::DeviceOrHostAddressConstKHR {
                        device_address: self.device.get_device_address(&instance_data),
                    },
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        let geometries = [geometry];
        let build_info = vk::AccelerationStructureBuildGeometryInfoKHR {
            ty: vk::AccelerationStructureTypeKHR::TOP_LEVEL,
            mode: vk::BuildAccelerationStructureModeKHR::BUILD,
            scratch_data: vk::DeviceOrHostAddressKHR {
                device_address: self.device.get_device_address(&scratch_data),
            },
            dst_acceleration_structure: acceleration_structure.raw,
            ..Default::default()
        }
        .geometries(&geometries);

        let rt = self.device.ray_tracing.as_ref().unwrap();
        unsafe {
            rt.acceleration_structure.cmd_build_acceleration_structures(
                self.raw,
                &[build_info],
                &[&[build_range_info]],
            );
        }
    }
}

impl Drop for super::AccelerationStructureCommandEncoder<'_> {
    fn drop(&mut self) {
        end_pass(self.device, self.raw);
    }
}

impl<'a> super::ComputeCommandEncoder<'a> {
    pub fn with<'b, 'p>(
        &'b mut self,
        pipeline: &'p super::ComputePipeline,
    ) -> super::PipelineEncoder<'b, 'p> {
        let bind_point = vk::PipelineBindPoint::COMPUTE;
        unsafe {
            self.device
                .core
                .cmd_bind_pipeline(self.cmd_buf.raw, bind_point, pipeline.raw)
        };
        super::PipelineEncoder {
            cmd_buf: self.cmd_buf,
            layout: &pipeline.layout,
            bind_point,
            device: self.device,
            update_data: self.update_data,
        }
    }
}

impl Drop for super::ComputeCommandEncoder<'_> {
    fn drop(&mut self) {
        end_pass(self.device, self.cmd_buf.raw);
    }
}

impl<'a> super::RenderCommandEncoder<'a> {
    pub fn with<'b, 'p>(
        &'b mut self,
        pipeline: &'p super::RenderPipeline,
    ) -> super::PipelineEncoder<'b, 'p> {
        let bind_point = vk::PipelineBindPoint::GRAPHICS;
        unsafe {
            self.device
                .core
                .cmd_bind_pipeline(self.cmd_buf.raw, bind_point, pipeline.raw)
        };
        super::PipelineEncoder {
            cmd_buf: self.cmd_buf,
            layout: &pipeline.layout,
            bind_point,
            device: self.device,
            update_data: self.update_data,
        }
    }
}

impl Drop for super::RenderCommandEncoder<'_> {
    fn drop(&mut self) {
        unsafe {
            self.device
                .dynamic_rendering
                .cmd_end_rendering(self.cmd_buf.raw)
        };
        end_pass(self.device, self.cmd_buf.raw);
    }
}

impl crate::ScissorRect {
    const fn to_vk(&self) -> vk::Rect2D {
        vk::Rect2D {
            offset: vk::Offset2D {
                x: self.x,
                y: self.y,
            },
            extent: vk::Extent2D {
                width: self.w,
                height: self.h,
            },
        }
    }
}

impl crate::Viewport {
    fn to_vk(&self) -> vk::Viewport {
        vk::Viewport {
            x: self.x,
            y: self.y + self.h, // shift since always flipped y and vulkan >= 1.1
            width: self.w,
            height: -self.h, // flip Y
            min_depth: self.depth.start,
            max_depth: self.depth.end,
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderEncoder for super::RenderCommandEncoder<'_> {
    fn set_scissor_rect(&mut self, rect: &crate::ScissorRect) {
        let vk_scissor = rect.to_vk();
        unsafe {
            self.device
                .core
                .cmd_set_scissor(self.cmd_buf.raw, 0, &[vk_scissor])
        };
    }

    fn set_viewport(&mut self, viewport: &crate::Viewport) {
        let vk_viewport = viewport.to_vk();
        unsafe {
            self.device
                .core
                .cmd_set_viewport(self.cmd_buf.raw, 0, &[vk_viewport])
        };
    }

    fn set_stencil_reference(&mut self, reference: u32) {
        unsafe {
            self.device.core.cmd_set_stencil_reference(
                self.cmd_buf.raw,
                vk::StencilFaceFlags::FRONT_AND_BACK,
                reference,
            )
        };
    }
}

#[hidden_trait::expose]
impl crate::traits::PipelineEncoder for super::PipelineEncoder<'_, '_> {
    fn bind<D: crate::ShaderData>(&mut self, group: u32, data: &D) {
        let dsl = &self.layout.descriptor_set_layouts[group as usize];
        if !dsl.is_empty() {
            self.update_data.clear();
            self.update_data.resize(dsl.template_size as usize, 0);
            data.fill(super::PipelineContext {
                update_data: self.update_data.as_mut_slice(),
                template_offsets: &dsl.template_offsets,
            });
        }

        let vk_set = self
            .device
            .allocate_descriptor_set(&mut self.cmd_buf.descriptor_pool, dsl);
        unsafe {
            if !dsl.is_empty() {
                self.device.core.update_descriptor_set_with_template(
                    vk_set,
                    dsl.update_template,
                    self.update_data.as_ptr() as *const _,
                );
            }
            self.device.core.cmd_bind_descriptor_sets(
                self.cmd_buf.raw,
                self.bind_point,
                self.layout.raw,
                group,
                &[vk_set],
                &[],
            );
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::ComputePipelineEncoder for super::PipelineEncoder<'_, '_> {
    type BufferPiece = crate::BufferPiece;

    fn dispatch(&mut self, groups: [u32; 3]) {
        unsafe {
            self.device
                .core
                .cmd_dispatch(self.cmd_buf.raw, groups[0], groups[1], groups[2])
        };
    }
    fn dispatch_indirect(&mut self, indirect_buf: crate::BufferPiece) {
        unsafe {
            self.device.core.cmd_dispatch_indirect(
                self.cmd_buf.raw,
                indirect_buf.buffer.raw,
                indirect_buf.offset,
            )
        };
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderEncoder for super::PipelineEncoder<'_, '_> {
    fn set_scissor_rect(&mut self, rect: &crate::ScissorRect) {
        let vk_scissor = rect.to_vk();
        unsafe {
            self.device
                .core
                .cmd_set_scissor(self.cmd_buf.raw, 0, &[vk_scissor])
        };
    }

    fn set_viewport(&mut self, viewport: &crate::Viewport) {
        let vk_viewport = viewport.to_vk();
        unsafe {
            self.device
                .core
                .cmd_set_viewport(self.cmd_buf.raw, 0, &[vk_viewport])
        };
    }

    fn set_stencil_reference(&mut self, reference: u32) {
        unsafe {
            self.device.core.cmd_set_stencil_reference(
                self.cmd_buf.raw,
                vk::StencilFaceFlags::FRONT_AND_BACK,
                reference,
            )
        };
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderPipelineEncoder for super::PipelineEncoder<'_, '_> {
    type BufferPiece = crate::BufferPiece;

    fn bind_vertex(&mut self, index: u32, vertex_buf: crate::BufferPiece) {
        unsafe {
            self.device.core.cmd_bind_vertex_buffers(
                self.cmd_buf.raw,
                index,
                &[vertex_buf.buffer.raw],
                &[vertex_buf.offset],
            );
        }
    }

    fn draw(
        &mut self,
        start_vertex: u32,
        vertex_count: u32,
        start_instance: u32,
        instance_count: u32,
    ) {
        unsafe {
            self.device.core.cmd_draw(
                self.cmd_buf.raw,
                vertex_count,
                instance_count,
                start_vertex,
                start_instance,
            );
        }
    }

    fn draw_indexed(
        &mut self,
        index_buf: crate::BufferPiece,
        index_type: crate::IndexType,
        index_count: u32,
        base_vertex: i32,
        start_instance: u32,
        instance_count: u32,
    ) {
        let raw_index_type = super::map_index_type(index_type);
        unsafe {
            self.device.core.cmd_bind_index_buffer(
                self.cmd_buf.raw,
                index_buf.buffer.raw,
                index_buf.offset,
                raw_index_type,
            );
            self.device.core.cmd_draw_indexed(
                self.cmd_buf.raw,
                index_count,
                instance_count,
                0,
                base_vertex,
                start_instance,
            );
        }
    }

    fn draw_indirect(&mut self, indirect_buf: crate::BufferPiece) {
        unsafe {
            self.device.core.cmd_draw_indirect(
                self.cmd_buf.raw,
                indirect_buf.buffer.raw,
                indirect_buf.offset,
                1,
                0,
            );
        }
    }

    fn draw_indexed_indirect(
        &mut self,
        index_buf: crate::BufferPiece,
        index_type: crate::IndexType,
        indirect_buf: crate::BufferPiece,
    ) {
        let raw_index_type = super::map_index_type(index_type);
        unsafe {
            self.device.core.cmd_bind_index_buffer(
                self.cmd_buf.raw,
                index_buf.buffer.raw,
                index_buf.offset,
                raw_index_type,
            );
            self.device.core.cmd_draw_indexed_indirect(
                self.cmd_buf.raw,
                indirect_buf.buffer.raw,
                indirect_buf.offset,
                1,
                0,
            );
        }
    }
}
