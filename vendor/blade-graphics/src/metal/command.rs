use objc2_foundation::{NSArray, NSRange, NSString};
use objc2_metal::{
    self as metal, MTLAccelerationStructureCommandEncoder as _, MTLBlitCommandEncoder,
    MTLCommandBuffer as _, MTLCommandEncoder, MTLComputeCommandEncoder as _,
    MTLCounterSampleBuffer, MTLRenderCommandEncoder,
};
use std::{marker::PhantomData, mem, ptr::NonNull, slice, time::Duration};

impl<T: bytemuck::Pod> crate::ShaderBindable for T {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        let slot = ctx.targets[index as usize] as _;
        let size = mem::size_of::<T>();
        unsafe {
            let ptr = NonNull::new_unchecked(self as *const _ as *mut _);
            if let Some(encoder) = ctx.vs_encoder {
                encoder.setVertexBytes_length_atIndex(ptr, size, slot);
            }
            if let Some(encoder) = ctx.fs_encoder {
                encoder.setFragmentBytes_length_atIndex(ptr, size, slot);
            }
            if let Some(encoder) = ctx.cs_encoder {
                encoder.setBytes_length_atIndex(ptr, size, slot);
            }
        }
    }
}
impl crate::ShaderBindable for super::TextureView {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        let slot = ctx.targets[index as usize] as _;
        let value = Some(self.as_ref());
        unsafe {
            if let Some(encoder) = ctx.vs_encoder {
                encoder.setVertexTexture_atIndex(value, slot);
            }
            if let Some(encoder) = ctx.fs_encoder {
                encoder.setFragmentTexture_atIndex(value, slot);
            }
            if let Some(encoder) = ctx.cs_encoder {
                encoder.setTexture_atIndex(value, slot);
            }
        }
    }
}
impl<'a, const N: crate::ResourceIndex> crate::ShaderBindable for &'a crate::TextureArray<N> {
    fn bind_to(&self, _ctx: &mut super::PipelineContext, _index: u32) {
        unimplemented!()
    }
}
impl crate::ShaderBindable for super::Sampler {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        let slot = ctx.targets[index as usize] as _;
        let value = Some(self.as_ref());
        unsafe {
            if let Some(encoder) = ctx.vs_encoder {
                encoder.setVertexSamplerState_atIndex(value, slot);
            }
            if let Some(encoder) = ctx.fs_encoder {
                encoder.setFragmentSamplerState_atIndex(value, slot);
            }
            if let Some(encoder) = ctx.cs_encoder {
                encoder.setSamplerState_atIndex(value, slot);
            }
        }
    }
}
impl crate::ShaderBindable for crate::BufferPiece {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        let slot = ctx.targets[index as usize] as _;
        let value = Some(self.buffer.as_ref());
        unsafe {
            if let Some(encoder) = ctx.vs_encoder {
                encoder.setVertexBuffer_offset_atIndex(value, self.offset as usize, slot);
            }
            if let Some(encoder) = ctx.fs_encoder {
                encoder.setFragmentBuffer_offset_atIndex(value, self.offset as usize, slot);
            }
            if let Some(encoder) = ctx.cs_encoder {
                encoder.setBuffer_offset_atIndex(value, self.offset as usize, slot);
            }
        }
    }
}
impl<'a, const N: crate::ResourceIndex> crate::ShaderBindable for &'a crate::BufferArray<N> {
    fn bind_to(&self, _ctx: &mut super::PipelineContext, _index: u32) {
        unimplemented!()
    }
}
impl crate::ShaderBindable for crate::AccelerationStructure {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        let slot = ctx.targets[index as usize] as _;
        let value = Some(self.as_ref());
        unsafe {
            if let Some(encoder) = ctx.vs_encoder {
                encoder.setVertexAccelerationStructure_atBufferIndex(value, slot);
            }
            if let Some(encoder) = ctx.fs_encoder {
                encoder.setFragmentAccelerationStructure_atBufferIndex(value, slot);
            }
            if let Some(encoder) = ctx.cs_encoder {
                encoder.setAccelerationStructure_atBufferIndex(value, slot);
            }
        }
    }
}

impl super::TimingData {
    fn add(&mut self, label: &str) -> usize {
        let counter_index = self.pass_names.len() * 2;
        self.pass_names.push(label.to_string());
        counter_index
    }
}

impl super::CommandEncoder {
    fn begin_pass(&mut self, label: &str) {
        if self.enable_debug_groups {
            //HACK: close the previous group
            if self.has_open_debug_group {
                self.raw.as_mut().unwrap().popDebugGroup();
            } else {
                self.has_open_debug_group = true;
            }
            let string = NSString::from_str(label);
            self.raw.as_mut().unwrap().pushDebugGroup(&string);
        }
    }

    pub(super) fn finish(&mut self) -> super::RawCommandBuffer {
        if self.has_open_debug_group {
            self.raw.as_mut().unwrap().popDebugGroup();
        }
        self.raw.take().unwrap()
    }

    pub fn transfer(&mut self, label: &str) -> super::TransferCommandEncoder {
        self.begin_pass(label);
        let raw = objc2::rc::autoreleasepool(|_| unsafe {
            let descriptor = metal::MTLBlitPassDescriptor::new();
            if let Some(ref mut td_array) = self.timing_datas {
                let td = td_array.first_mut().unwrap();
                let counter_index = td.add(label);
                let sba = descriptor
                    .sampleBufferAttachments()
                    .objectAtIndexedSubscript(0);
                sba.setSampleBuffer(Some(&td.sample_buffer));
                sba.setStartOfEncoderSampleIndex(counter_index);
                sba.setEndOfEncoderSampleIndex(counter_index + 1);
            }

            self.raw
                .as_mut()
                .unwrap()
                .blitCommandEncoderWithDescriptor(&descriptor)
                .unwrap()
        });
        super::TransferCommandEncoder {
            raw,
            phantom: PhantomData,
        }
    }

    pub fn acceleration_structure(
        &mut self,
        label: &str,
    ) -> super::AccelerationStructureCommandEncoder {
        let raw = objc2::rc::autoreleasepool(|_| unsafe {
            let descriptor = metal::MTLAccelerationStructurePassDescriptor::new();

            if let Some(ref mut td_array) = self.timing_datas {
                let td = td_array.first_mut().unwrap();
                let counter_index = td.add(label);
                let sba = descriptor
                    .sampleBufferAttachments()
                    .objectAtIndexedSubscript(0);
                sba.setSampleBuffer(Some(&td.sample_buffer));
                sba.setStartOfEncoderSampleIndex(counter_index);
                sba.setEndOfEncoderSampleIndex(counter_index + 1);
            }

            self.raw
                .as_mut()
                .unwrap()
                .accelerationStructureCommandEncoderWithDescriptor(&descriptor)
        });
        super::AccelerationStructureCommandEncoder {
            raw,
            phantom: PhantomData,
        }
    }

    pub fn compute(&mut self, label: &str) -> super::ComputeCommandEncoder {
        let raw = objc2::rc::autoreleasepool(|_| unsafe {
            let descriptor = metal::MTLComputePassDescriptor::new();
            if self.enable_dispatch_type {
                descriptor.setDispatchType(metal::MTLDispatchType::Concurrent);
            }

            if let Some(ref mut td_array) = self.timing_datas {
                let td = td_array.first_mut().unwrap();
                let counter_index = td.add(label);
                let sba = descriptor
                    .sampleBufferAttachments()
                    .objectAtIndexedSubscript(0);
                sba.setSampleBuffer(Some(&td.sample_buffer));
                sba.setStartOfEncoderSampleIndex(counter_index);
                sba.setEndOfEncoderSampleIndex(counter_index + 1);
            }

            self.raw
                .as_mut()
                .unwrap()
                .computeCommandEncoderWithDescriptor(&descriptor)
                .unwrap()
        });
        super::ComputeCommandEncoder {
            raw,
            phantom: PhantomData,
        }
    }

    pub fn render(
        &mut self,
        label: &str,
        targets: crate::RenderTargetSet,
    ) -> super::RenderCommandEncoder {
        let raw = objc2::rc::autoreleasepool(|_| {
            let descriptor = unsafe { metal::MTLRenderPassDescriptor::new() };

            for (i, rt) in targets.colors.iter().enumerate() {
                let at_descriptor =
                    unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(i) };
                at_descriptor.setTexture(Some(rt.view.as_ref()));

                let load_action = match rt.init_op {
                    crate::InitOp::Load => metal::MTLLoadAction::Load,
                    crate::InitOp::Clear(color) => {
                        let clear_color = map_clear_color(color);
                        at_descriptor.setClearColor(clear_color);
                        metal::MTLLoadAction::Clear
                    }
                    crate::InitOp::DontCare => metal::MTLLoadAction::DontCare,
                };
                at_descriptor.setLoadAction(load_action);

                let store_action = match rt.finish_op {
                    crate::FinishOp::Store | crate::FinishOp::Ignore => {
                        metal::MTLStoreAction::Store
                    }
                    crate::FinishOp::Discard => metal::MTLStoreAction::DontCare,
                    crate::FinishOp::ResolveTo(ref view) => {
                        at_descriptor.setResolveTexture(Some(view.as_ref()));
                        metal::MTLStoreAction::MultisampleResolve
                    }
                };
                at_descriptor.setStoreAction(store_action);
            }

            if let Some(ref rt) = targets.depth_stencil {
                if rt.view.aspects.contains(crate::TexelAspects::DEPTH) {
                    let at_descriptor = descriptor.depthAttachment();
                    at_descriptor.setTexture(Some(rt.view.as_ref()));
                    let load_action = match rt.init_op {
                        crate::InitOp::Load => metal::MTLLoadAction::Load,
                        crate::InitOp::Clear(color) => {
                            let clear_depth = color.depth_clear_value();
                            at_descriptor.setClearDepth(clear_depth as f64);
                            metal::MTLLoadAction::Clear
                        }
                        crate::InitOp::DontCare => metal::MTLLoadAction::DontCare,
                    };
                    let store_action = match rt.finish_op {
                        crate::FinishOp::Store | crate::FinishOp::Ignore => {
                            metal::MTLStoreAction::Store
                        }
                        crate::FinishOp::Discard => metal::MTLStoreAction::DontCare,
                        crate::FinishOp::ResolveTo(_) => panic!("Can't resolve depth texture"),
                    };
                    at_descriptor.setLoadAction(load_action);
                    at_descriptor.setStoreAction(store_action);
                }

                if rt.view.aspects.contains(crate::TexelAspects::STENCIL) {
                    let at_descriptor = descriptor.stencilAttachment();
                    at_descriptor.setTexture(Some(rt.view.as_ref()));

                    let load_action = match rt.init_op {
                        crate::InitOp::Load => metal::MTLLoadAction::Load,
                        crate::InitOp::Clear(color) => {
                            let clear_stencil = color.stencil_clear_value();
                            at_descriptor.setClearStencil(clear_stencil);
                            metal::MTLLoadAction::Clear
                        }
                        crate::InitOp::DontCare => metal::MTLLoadAction::DontCare,
                    };
                    let store_action = match rt.finish_op {
                        crate::FinishOp::Store | crate::FinishOp::Ignore => {
                            metal::MTLStoreAction::Store
                        }
                        crate::FinishOp::Discard => metal::MTLStoreAction::DontCare,
                        crate::FinishOp::ResolveTo(_) => panic!("Can't resolve stencil texture"),
                    };

                    at_descriptor.setLoadAction(load_action);
                    at_descriptor.setStoreAction(store_action);
                }
            }

            if let Some(ref mut td_array) = self.timing_datas {
                let td = td_array.first_mut().unwrap();
                let counter_index = td.add(label);
                unsafe {
                    let sba = descriptor
                        .sampleBufferAttachments()
                        .objectAtIndexedSubscript(0);
                    sba.setSampleBuffer(Some(&td.sample_buffer));
                    sba.setStartOfVertexSampleIndex(counter_index);
                    sba.setEndOfFragmentSampleIndex(counter_index + 1);
                }
            }

            self.raw
                .as_mut()
                .unwrap()
                .renderCommandEncoderWithDescriptor(&descriptor)
                .unwrap()
        });

        super::RenderCommandEncoder {
            raw,
            phantom: PhantomData,
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::CommandEncoder for super::CommandEncoder {
    type Texture = super::Texture;
    type Frame = super::Frame;

    fn start(&mut self) {
        if let Some(ref mut td_array) = self.timing_datas {
            self.timings.clear();
            td_array.rotate_left(1);
            let td = td_array.first_mut().unwrap();
            if !td.pass_names.is_empty() {
                let ns_data = unsafe {
                    td.sample_buffer
                        .resolveCounterRange(NSRange::new(0, td.pass_names.len() * 2))
                        .unwrap()
                };
                let counters = unsafe {
                    slice::from_raw_parts(
                        ns_data.as_bytes_unchecked().as_ptr() as *const u64,
                        ns_data.len() / mem::size_of::<u64>(),
                    )
                };
                for (name, chunk) in td.pass_names.drain(..).zip(counters.chunks(2)) {
                    let duration = Duration::from_nanos(chunk[1] - chunk[0]);
                    self.timings.push((name, duration));
                }
            }
        }

        let queue = self.queue.lock().unwrap();
        self.raw = Some(objc2::rc::autoreleasepool(|_| unsafe {
            use metal::MTLCommandQueue as _;
            let cmd_buf = queue.commandBufferWithUnretainedReferences().unwrap();
            if !self.name.is_empty() {
                cmd_buf.setLabel(Some(&NSString::from_str(&self.name)));
            }
            cmd_buf
        }));
        self.has_open_debug_group = false;
    }

    fn init_texture(&mut self, _texture: super::Texture) {}

    fn present(&mut self, frame: super::Frame) {
        self.raw.as_mut().unwrap().presentDrawable(&frame.drawable);
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
        let range = NSRange {
            location: dst.offset as usize,
            length: size as usize,
        };
        self.raw
            .fillBuffer_range_value(dst.buffer.as_ref(), range, value);
    }

    fn copy_buffer_to_buffer(
        &mut self,
        src: crate::BufferPiece,
        dst: crate::BufferPiece,
        size: u64,
    ) {
        unsafe {
            self.raw
                .copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                    src.buffer.as_ref(),
                    src.offset as usize,
                    dst.buffer.as_ref(),
                    dst.offset as usize,
                    size as usize,
                )
        };
    }
    fn copy_texture_to_texture(
        &mut self,
        src: crate::TexturePiece,
        dst: crate::TexturePiece,
        size: crate::Extent,
    ) {
        unsafe {
            self.raw.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                src.texture.as_ref(),
                src.array_layer as usize,
                src.mip_level as usize,
                map_origin(&src.origin),
                map_extent(&size),
                dst.texture.as_ref(),
                dst.array_layer as usize,
                dst.mip_level as usize,
                map_origin(&dst.origin),
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
        unsafe {
            self.raw.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin_options(
                src.buffer.as_ref(),
                src.offset as usize,
                bytes_per_row as usize,
                0,
                map_extent(&size),
                dst.texture.as_ref(),
                dst.array_layer as usize,
                dst.mip_level as usize,
                map_origin(&dst.origin),
                metal::MTLBlitOption::empty(),
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
        unsafe {
            self.raw.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage_options(
                src.texture.as_ref(),
                src.array_layer as usize,
                src.mip_level as usize,
                map_origin(&src.origin),
                map_extent(&size),
                dst.buffer.as_ref(),
                dst.offset as usize,
                bytes_per_row as usize,
                0,
                metal::MTLBlitOption::empty(),
            )
        };
    }
}

impl Drop for super::TransferCommandEncoder<'_> {
    fn drop(&mut self) {
        self.raw.endEncoding();
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
        let descriptor = super::make_bottom_level_acceleration_structure_desc(meshes);
        self.raw
            .buildAccelerationStructure_descriptor_scratchBuffer_scratchBufferOffset(
                acceleration_structure.as_ref(),
                &descriptor,
                scratch_data.buffer.as_ref(),
                scratch_data.offset as usize,
            );
    }

    fn build_top_level(
        &mut self,
        acceleration_structure: super::AccelerationStructure,
        bottom_level: &[super::AccelerationStructure],
        instance_count: u32,
        instance_data: crate::BufferPiece,
        scratch_data: crate::BufferPiece,
    ) {
        let mut primitive_acceleration_structures = Vec::with_capacity(bottom_level.len());
        for blas in bottom_level {
            primitive_acceleration_structures.push(blas.as_retained());
        }
        let descriptor = metal::MTLInstanceAccelerationStructureDescriptor::descriptor();
        descriptor.setInstancedAccelerationStructures(Some(&NSArray::from_retained_slice(
            &primitive_acceleration_structures,
        )));
        descriptor.setInstanceCount(instance_count as usize);
        unsafe {
            descriptor.setInstanceDescriptorType(
                metal::MTLAccelerationStructureInstanceDescriptorType::UserID,
            );
            descriptor.setInstanceDescriptorBuffer(Some(instance_data.buffer.as_ref()));
            descriptor.setInstanceDescriptorBufferOffset(instance_data.offset as usize);
        }

        self.raw
            .buildAccelerationStructure_descriptor_scratchBuffer_scratchBufferOffset(
                acceleration_structure.as_ref(),
                &descriptor,
                scratch_data.buffer.as_ref(),
                scratch_data.offset as usize,
            );
    }
}

impl Drop for super::AccelerationStructureCommandEncoder<'_> {
    fn drop(&mut self) {
        self.raw.endEncoding();
    }
}

impl super::ComputeCommandEncoder<'_> {
    pub fn with<'p>(
        &'p mut self,
        pipeline: &'p super::ComputePipeline,
    ) -> super::ComputePipelineContext<'p> {
        self.raw.pushDebugGroup(&NSString::from_str(&pipeline.name));
        self.raw.setComputePipelineState(&pipeline.raw);
        if let Some(index) = pipeline.layout.sizes_buffer_slot {
            //TODO: get real sizes? shouldn't matter without bounds checks
            let runtime_sizes = [0u8; 8];
            unsafe {
                self.raw.setBytes_length_atIndex(
                    NonNull::new(runtime_sizes.as_ptr() as *const _ as *mut _).unwrap(),
                    runtime_sizes.len(),
                    index as _,
                );
            }
        }
        for (index, &size) in pipeline.wg_memory_sizes.iter().enumerate() {
            unsafe {
                self.raw
                    .setThreadgroupMemoryLength_atIndex(size as _, index);
            }
        }

        super::ComputePipelineContext {
            encoder: self.raw.as_ref(),
            wg_size: pipeline.wg_size,
            group_mappings: &pipeline.layout.group_mappings,
        }
    }
}

impl Drop for super::ComputeCommandEncoder<'_> {
    fn drop(&mut self) {
        self.raw.endEncoding();
    }
}

impl crate::ScissorRect {
    const fn to_metal(&self) -> metal::MTLScissorRect {
        metal::MTLScissorRect {
            x: self.x as _,
            y: self.y as _,
            width: self.w as _,
            height: self.h as _,
        }
    }
}
impl crate::Viewport {
    const fn to_metal(&self) -> metal::MTLViewport {
        metal::MTLViewport {
            originX: self.x as _,
            originY: self.y as _,
            width: self.w as _,
            height: self.h as _,
            znear: self.depth.start as _,
            // TODO: broken on some Intel GPUs
            // https://github.com/gfx-rs/wgpu/blob/ee3ae0e549fe01c4b699cf68f9b67ae8ea807564/wgpu-hal/src/metal/mod.rs#L298
            zfar: self.depth.end as _,
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderEncoder for super::RenderCommandEncoder<'_> {
    fn set_scissor_rect(&mut self, rect: &crate::ScissorRect) {
        self.raw.setScissorRect(rect.to_metal());
    }

    fn set_viewport(&mut self, viewport: &crate::Viewport) {
        self.raw.setViewport(viewport.to_metal());
    }

    fn set_stencil_reference(&mut self, stencil_reference: u32) {
        self.raw.setStencilReferenceValue(stencil_reference);
    }
}

impl super::RenderCommandEncoder<'_> {
    pub fn with<'p>(
        &'p mut self,
        pipeline: &'p super::RenderPipeline,
    ) -> super::RenderPipelineContext<'p> {
        self.raw.pushDebugGroup(&NSString::from_str(&pipeline.name));
        self.raw.setRenderPipelineState(&pipeline.raw);
        if let Some(index) = pipeline.layout.sizes_buffer_slot {
            //TODO: get real sizes
            let runtime_sizes = [0u8; 8];
            unsafe {
                self.raw.setVertexBytes_length_atIndex(
                    NonNull::new(runtime_sizes.as_ptr() as *const _ as *mut _).unwrap(),
                    runtime_sizes.len(),
                    index as _,
                );
                self.raw.setFragmentBytes_length_atIndex(
                    NonNull::new(runtime_sizes.as_ptr() as *const _ as *mut _).unwrap(),
                    runtime_sizes.len(),
                    index as _,
                );
            }
        }

        self.raw.setFrontFacingWinding(pipeline.front_winding);
        self.raw.setCullMode(pipeline.cull_mode);
        self.raw.setTriangleFillMode(pipeline.triangle_fill_mode);
        self.raw.setDepthClipMode(pipeline.depth_clip_mode);
        if let Some((ref state, bias)) = pipeline.depth_stencil {
            self.raw.setDepthStencilState(Some(state));
            self.raw.setDepthBias_slopeScale_clamp(
                bias.constant as f32,
                bias.slope_scale,
                bias.clamp,
            );
        }

        super::RenderPipelineContext {
            encoder: self.raw.as_ref(),
            primitive_type: pipeline.primitive_type,
            group_mappings: &pipeline.layout.group_mappings,
        }
    }
}

impl Drop for super::RenderCommandEncoder<'_> {
    fn drop(&mut self) {
        self.raw.endEncoding();
    }
}

#[hidden_trait::expose]
impl crate::traits::PipelineEncoder for super::ComputePipelineContext<'_> {
    fn bind<D: crate::ShaderData>(&mut self, group: u32, data: &D) {
        let info = &self.group_mappings[group as usize];

        data.fill(super::PipelineContext {
            cs_encoder: if info.visibility.contains(crate::ShaderVisibility::COMPUTE) {
                Some(self.encoder.as_ref())
            } else {
                None
            },
            vs_encoder: None,
            fs_encoder: None,
            targets: &info.targets,
        });
    }
}

#[hidden_trait::expose]
impl crate::traits::ComputePipelineEncoder for super::ComputePipelineContext<'_> {
    type BufferPiece = crate::BufferPiece;

    fn dispatch(&mut self, groups: [u32; 3]) {
        let raw_count = metal::MTLSize {
            width: groups[0] as usize,
            height: groups[1] as usize,
            depth: groups[2] as usize,
        };
        self.encoder
            .dispatchThreadgroups_threadsPerThreadgroup(raw_count, self.wg_size);
    }

    fn dispatch_indirect(&mut self, indirect_buf: crate::BufferPiece) {
        unsafe {
            self.encoder
                .dispatchThreadgroupsWithIndirectBuffer_indirectBufferOffset_threadsPerThreadgroup(
                    indirect_buf.buffer.as_ref(),
                    indirect_buf.offset as usize,
                    self.wg_size,
                );
        }
    }
}

impl Drop for super::ComputePipelineContext<'_> {
    fn drop(&mut self) {
        self.encoder.popDebugGroup();
    }
}

#[hidden_trait::expose]
impl crate::traits::PipelineEncoder for super::RenderPipelineContext<'_> {
    fn bind<D: crate::ShaderData>(&mut self, group: u32, data: &D) {
        let info = &self.group_mappings[group as usize];

        data.fill(super::PipelineContext {
            cs_encoder: None,
            vs_encoder: if info.visibility.contains(crate::ShaderVisibility::VERTEX) {
                Some(self.encoder.as_ref())
            } else {
                None
            },
            fs_encoder: if info.visibility.contains(crate::ShaderVisibility::FRAGMENT) {
                Some(self.encoder.as_ref())
            } else {
                None
            },
            targets: &info.targets,
        });
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderEncoder for super::RenderPipelineContext<'_> {
    fn set_scissor_rect(&mut self, rect: &crate::ScissorRect) {
        self.encoder.setScissorRect(rect.to_metal());
    }
    fn set_viewport(&mut self, viewport: &crate::Viewport) {
        self.encoder.setViewport(viewport.to_metal());
    }
    fn set_stencil_reference(&mut self, stencil_reference: u32) {
        self.encoder.setStencilReferenceValue(stencil_reference);
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderPipelineEncoder for super::RenderPipelineContext<'_> {
    type BufferPiece = crate::BufferPiece;

    fn bind_vertex(&mut self, index: u32, vertex_buf: crate::BufferPiece) {
        unsafe {
            self.encoder.setVertexBuffer_offset_atIndex(
                Some(vertex_buf.buffer.as_ref()),
                vertex_buf.offset as usize,
                index as usize,
            );
        }
    }

    fn draw(
        &mut self,
        first_vertex: u32,
        vertex_count: u32,
        first_instance: u32,
        instance_count: u32,
    ) {
        unsafe {
            if first_instance != 0 {
                self.encoder
                    .drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                        self.primitive_type,
                        first_vertex as _,
                        vertex_count as _,
                        instance_count as _,
                        first_instance as _,
                    );
            } else if instance_count != 1 {
                self.encoder
                    .drawPrimitives_vertexStart_vertexCount_instanceCount(
                        self.primitive_type,
                        first_vertex as _,
                        vertex_count as _,
                        instance_count as _,
                    );
            } else {
                self.encoder.drawPrimitives_vertexStart_vertexCount(
                    self.primitive_type,
                    first_vertex as _,
                    vertex_count as _,
                );
            }
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
            if base_vertex != 0 || start_instance != 0 {
                self.encoder
                .drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount_baseVertex_baseInstance(
                    self.primitive_type,
                    index_count as _,
                    raw_index_type,
                    index_buf.buffer.as_ref(),
                    index_buf.offset as usize,
                    instance_count as _,
                    base_vertex as _,
                    start_instance as _,
                );
            } else if instance_count != 1 {
                self.encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount(
                self.primitive_type,
                index_count as _,
                raw_index_type,
                index_buf.buffer.as_ref(),
                index_buf.offset as usize,
                instance_count as _,
            );
            } else {
                self.encoder
                    .drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
                        self.primitive_type,
                        index_count as _,
                        raw_index_type,
                        index_buf.buffer.as_ref(),
                        index_buf.offset as usize,
                    );
            }
        }
    }

    fn draw_indirect(&mut self, indirect_buf: crate::BufferPiece) {
        unsafe {
            self.encoder
                .drawPrimitives_indirectBuffer_indirectBufferOffset(
                    self.primitive_type,
                    indirect_buf.buffer.as_ref(),
                    indirect_buf.offset as usize,
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
            self.encoder.drawIndexedPrimitives_indexType_indexBuffer_indexBufferOffset_indirectBuffer_indirectBufferOffset(
            self.primitive_type,
            raw_index_type,
            index_buf.buffer.as_ref(),
            index_buf.offset as usize,
            indirect_buf.buffer.as_ref(),
            indirect_buf.offset as usize,
        );
        }
    }
}

impl Drop for super::RenderPipelineContext<'_> {
    fn drop(&mut self) {
        self.encoder.popDebugGroup();
    }
}

fn map_origin(origin: &[u32; 3]) -> metal::MTLOrigin {
    metal::MTLOrigin {
        x: origin[0] as usize,
        y: origin[1] as usize,
        z: origin[2] as usize,
    }
}

fn map_extent(extent: &crate::Extent) -> metal::MTLSize {
    metal::MTLSize {
        width: extent.width as usize,
        height: extent.height as usize,
        depth: extent.depth as usize,
    }
}

fn map_clear_color(color: crate::TextureColor) -> metal::MTLClearColor {
    match color {
        crate::TextureColor::TransparentBlack => metal::MTLClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 0.0,
        },
        crate::TextureColor::OpaqueBlack => metal::MTLClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        },
        crate::TextureColor::White => metal::MTLClearColor {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            alpha: 1.0,
        },
    }
}
