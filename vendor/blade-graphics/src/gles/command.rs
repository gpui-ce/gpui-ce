use std::{str, time::Duration};

const COLOR_ATTACHMENTS: &[u32] = &[
    glow::COLOR_ATTACHMENT0,
    glow::COLOR_ATTACHMENT1,
    glow::COLOR_ATTACHMENT2,
    glow::COLOR_ATTACHMENT3,
];

impl<T: bytemuck::Pod> crate::ShaderBindable for T {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        let self_slice = bytemuck::bytes_of(self);
        let alignment = ctx.limits.uniform_buffer_alignment as usize;
        let rem = ctx.plain_data.len() % alignment;
        if rem != 0 {
            ctx.plain_data
                .resize(ctx.plain_data.len() - rem + alignment, 0);
        }
        let offset = ctx.plain_data.len() as u32;
        let size = super::round_up_uniform_size(self_slice.len() as u32);
        ctx.plain_data.extend_from_slice(self_slice);
        ctx.plain_data
            .extend((self_slice.len() as u32..size).map(|_| 0));

        for &slot in ctx.targets[index as usize].iter() {
            ctx.commands
                .push(super::Command::BindUniform { slot, offset, size });
        }
    }
}
impl crate::ShaderBindable for super::TextureView {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        let (texture, target) = self.inner.as_native();
        for &slot in ctx.targets[index as usize].iter() {
            ctx.commands.push(super::Command::BindTexture {
                slot,
                texture,
                target,
            });
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
        for &slot in ctx.targets[index as usize].iter() {
            ctx.commands.push(super::Command::BindSampler {
                slot,
                sampler: self.raw,
            });
        }
    }
}
impl crate::ShaderBindable for crate::BufferPiece {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        for &slot in ctx.targets[index as usize].iter() {
            ctx.commands.push(super::Command::BindBuffer {
                target: glow::SHADER_STORAGE_BUFFER,
                slot,
                buffer: (*self).into(),
                size: (self.buffer.size - self.offset) as u32,
            });
        }
    }
}
impl<'a, const N: crate::ResourceIndex> crate::ShaderBindable for &'a crate::BufferArray<N> {
    fn bind_to(&self, _ctx: &mut super::PipelineContext, _index: u32) {
        unimplemented!()
    }
}
impl crate::ShaderBindable for super::AccelerationStructure {
    fn bind_to(&self, ctx: &mut super::PipelineContext, index: u32) {
        for _ in ctx.targets[index as usize].iter() {
            unimplemented!()
        }
    }
}

impl super::CommandEncoder {
    fn begin_pass(&mut self, label: &str) {
        if self.needs_scopes {
            let start = self.string_data.len();
            self.string_data.extend_from_slice(label.as_bytes());
            self.commands.push(super::Command::PushScope {
                name_range: start..self.string_data.len(),
            });
        }
        if let Some(ref mut timing_datas) = self.timing_datas {
            let td = timing_datas.first_mut().unwrap();
            let id = td.pass_names.len();
            self.commands.push(super::Command::QueryCounter {
                query: td.queries[id],
            });
            td.pass_names.push(label.to_string());
        }
    }

    fn pass<P>(&mut self, kind: super::PassKind) -> super::PassEncoder<P> {
        super::PassEncoder {
            commands: &mut self.commands,
            plain_data: &mut self.plain_data,
            kind,
            invalidate_attachments: Vec::new(),
            pipeline: Default::default(),
            limits: &self.limits,
            has_scope: self.needs_scopes,
        }
    }

    pub(super) fn finish(&mut self, gl: &glow::Context) {
        use glow::HasContext as _;
        #[allow(trivial_casts)]
        if let Some(ref mut timing_datas) = self.timing_datas {
            {
                let td = timing_datas.first_mut().unwrap();
                let id = td.pass_names.len();
                self.commands.push(super::Command::QueryCounter {
                    query: td.queries[id],
                });
            }

            timing_datas.rotate_left(1);
            self.timings.clear();
            let td = timing_datas.first_mut().unwrap();
            if !td.pass_names.is_empty() {
                let mut prev = 0;
                unsafe {
                    gl.get_query_parameter_u64_with_offset(
                        td.queries[0],
                        glow::QUERY_RESULT,
                        &mut prev as *mut _ as usize,
                    );
                }
                for (pass_name, &query) in td.pass_names.drain(..).zip(td.queries[1..].iter()) {
                    let mut result: u64 = 0;
                    unsafe {
                        gl.get_query_parameter_u64_with_offset(
                            query,
                            glow::QUERY_RESULT,
                            &mut result as *mut _ as usize,
                        );
                    }
                    let time = Duration::from_nanos(result - prev);
                    self.timings.push((pass_name, time));
                    prev = result
                }
            }
        }
    }

    pub fn transfer(&mut self, label: &str) -> super::PassEncoder<()> {
        self.begin_pass(label);
        self.pass(super::PassKind::Transfer)
    }

    pub fn acceleration_structure(&mut self, _label: &str) -> super::PassEncoder<()> {
        unimplemented!()
    }

    pub fn compute(&mut self, label: &str) -> super::PassEncoder<super::ComputePipeline> {
        self.begin_pass(label);
        self.pass(super::PassKind::Compute)
    }

    pub fn render(
        &mut self,
        label: &str,
        targets: crate::RenderTargetSet,
    ) -> super::PassEncoder<super::RenderPipeline> {
        self.begin_pass(label);

        let mut target_size = [0u16; 2];
        let mut invalidate_attachments = Vec::new();
        for (i, rt) in targets.colors.iter().enumerate() {
            let attachment = glow::COLOR_ATTACHMENT0 + i as u32;
            target_size = rt.view.target_size;
            self.commands.push(super::Command::BindAttachment {
                attachment,
                view: rt.view,
            });
            if let crate::FinishOp::Discard = rt.finish_op {
                invalidate_attachments.push(attachment);
            }
            if let crate::FinishOp::ResolveTo(to) = rt.finish_op {
                self.commands
                    .push(super::Command::BlitFramebuffer { from: rt.view, to });
            }
        }
        if let Some(ref rt) = targets.depth_stencil {
            let attachment = match rt.view.aspects {
                crate::TexelAspects::DEPTH => glow::DEPTH_ATTACHMENT,
                crate::TexelAspects::STENCIL => glow::STENCIL_ATTACHMENT,
                _ => glow::DEPTH_STENCIL_ATTACHMENT,
            };
            target_size = rt.view.target_size;
            self.commands.push(super::Command::BindAttachment {
                attachment,
                view: rt.view,
            });
            if let crate::FinishOp::Discard = rt.finish_op {
                invalidate_attachments.push(attachment);
            }
        }

        self.commands.push(super::Command::SetDrawColorBuffers(
            targets.colors.len() as _
        ));
        self.commands
            .push(super::Command::SetViewport(crate::Viewport {
                x: 0.0,
                y: 0.0,
                w: target_size[0] as _,
                h: target_size[1] as _,
                depth: 0.0..1.0,
            }));
        self.commands
            .push(super::Command::SetScissor(crate::ScissorRect {
                x: 0,
                y: 0,
                w: target_size[0] as u32,
                h: target_size[1] as u32,
            }));

        // issue the clears
        for (i, rt) in targets.colors.iter().enumerate() {
            if let crate::InitOp::Clear(color) = rt.init_op {
                self.commands.push(super::Command::ClearColor {
                    draw_buffer: i as u32,
                    color,
                    ty: super::ColorType::Float, //TODO: get from the format
                });
            }
        }
        if let Some(ref rt) = targets.depth_stencil {
            if let crate::InitOp::Clear(color) = rt.init_op {
                self.commands.push(super::Command::ClearDepthStencil {
                    depth: if rt.view.aspects.contains(crate::TexelAspects::DEPTH) {
                        Some(color.depth_clear_value())
                    } else {
                        None
                    },
                    stencil: if rt.view.aspects.contains(crate::TexelAspects::STENCIL) {
                        Some(color.stencil_clear_value())
                    } else {
                        None
                    },
                });
            }
        }

        let mut pass = self.pass(super::PassKind::Render);
        pass.invalidate_attachments = invalidate_attachments;
        pass
    }
}

#[hidden_trait::expose]
impl crate::traits::CommandEncoder for super::CommandEncoder {
    type Texture = super::Texture;
    type Frame = super::Frame;

    fn start(&mut self) {
        self.commands.clear();
        self.plain_data.clear();
        self.string_data.clear();
        self.present_frames.clear();
    }

    fn init_texture(&mut self, _texture: super::Texture) {}

    fn present(&mut self, frame: super::Frame) {
        self.present_frames.push(frame.platform);
    }

    fn timings(&self) -> &crate::Timings {
        &self.timings
    }
}

impl super::PassEncoder<'_, super::ComputePipeline> {
    pub fn with<'b>(
        &'b mut self,
        pipeline: &'b super::ComputePipeline,
    ) -> super::PipelineEncoder<'b> {
        self.commands
            .push(super::Command::SetProgram(pipeline.inner.program));
        super::PipelineEncoder {
            commands: self.commands,
            plain_data: self.plain_data,
            group_mappings: &pipeline.inner.group_mappings,
            topology: 0,
            limits: self.limits,
            vertex_attributes: &[],
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderEncoder for super::PassEncoder<'_, super::RenderPipeline> {
    fn set_scissor_rect(&mut self, rect: &crate::ScissorRect) {
        self.commands.push(super::Command::SetScissor(rect.clone()));
    }

    fn set_viewport(&mut self, viewport: &crate::Viewport) {
        self.commands
            .push(super::Command::SetViewport(viewport.clone()));
    }

    fn set_stencil_reference(&mut self, reference: u32) {
        unimplemented!()
    }
}

impl super::PassEncoder<'_, super::RenderPipeline> {
    pub fn with<'b>(
        &'b mut self,
        pipeline: &'b super::RenderPipeline,
    ) -> super::PipelineEncoder<'b> {
        self.commands
            .push(super::Command::SetProgram(pipeline.inner.program));

        match &pipeline.inner.color_targets[..] {
            &[(blend_state, write_masks)] => self
                .commands
                .push(super::Command::SetAllColorTargets(blend_state, write_masks)),
            separate => self.commands.extend(separate.iter().zip(0..).map(
                |(&(blend_state, write_masks), i)| {
                    super::Command::SetSingleColorTarget(i, blend_state, write_masks)
                },
            )),
        }
        super::PipelineEncoder {
            commands: self.commands,
            plain_data: self.plain_data,
            group_mappings: &pipeline.inner.group_mappings,
            topology: map_primitive_topology(pipeline.topology),
            limits: self.limits,
            vertex_attributes: &pipeline.inner.vertex_attribute_infos,
        }
    }
}

impl<T> Drop for super::PassEncoder<'_, T> {
    fn drop(&mut self) {
        self.commands.push(super::Command::UnsetProgram);
        for attachment in self.invalidate_attachments.drain(..) {
            self.commands
                .push(super::Command::InvalidateAttachment(attachment));
        }
        match self.kind {
            super::PassKind::Transfer => {}
            super::PassKind::Compute => {
                self.commands.push(super::Command::ResetAllSamplers);
            }
            super::PassKind::Render => {
                self.commands.push(super::Command::ResetAllSamplers);
                self.commands.push(super::Command::ResetFramebuffer);
            }
        }
        if self.has_scope {
            self.commands.push(super::Command::PopScope);
        }
    }
}

#[hidden_trait::expose]
impl crate::traits::TransferEncoder for super::PassEncoder<'_, ()> {
    type BufferPiece = crate::BufferPiece;
    type TexturePiece = crate::TexturePiece;

    fn fill_buffer(&mut self, dst: crate::BufferPiece, size: u64, value: u8) {
        self.commands.push(super::Command::FillBuffer {
            dst: dst.into(),
            size,
            value,
        });
    }

    fn copy_buffer_to_buffer(
        &mut self,
        src: crate::BufferPiece,
        dst: crate::BufferPiece,
        size: u64,
    ) {
        self.commands.push(super::Command::CopyBufferToBuffer {
            src: src.into(),
            dst: dst.into(),
            size,
        });
    }
    fn copy_texture_to_texture(
        &mut self,
        src: crate::TexturePiece,
        dst: crate::TexturePiece,
        size: crate::Extent,
    ) {
        self.commands.push(super::Command::CopyTextureToTexture {
            src: src.into(),
            dst: dst.into(),
            size,
        });
    }

    fn copy_buffer_to_texture(
        &mut self,
        src: crate::BufferPiece,
        bytes_per_row: u32,
        dst: crate::TexturePiece,
        size: crate::Extent,
    ) {
        self.commands.push(super::Command::CopyBufferToTexture {
            src: src.into(),
            bytes_per_row,
            dst: dst.into(),
            size,
        });
    }

    fn copy_texture_to_buffer(
        &mut self,
        src: crate::TexturePiece,
        dst: crate::BufferPiece,
        bytes_per_row: u32,
        size: crate::Extent,
    ) {
        self.commands.push(super::Command::CopyTextureToBuffer {
            src: src.into(),
            dst: dst.into(),
            bytes_per_row,
            size,
        });
    }
}

#[hidden_trait::expose]
impl crate::traits::PipelineEncoder for super::PipelineEncoder<'_> {
    fn bind<D: crate::ShaderData>(&mut self, group: u32, data: &D) {
        data.fill(super::PipelineContext {
            commands: self.commands,
            plain_data: self.plain_data,
            targets: &self.group_mappings[group as usize].targets,
            limits: self.limits,
        });
    }
}

#[hidden_trait::expose]
impl crate::traits::ComputePipelineEncoder for super::PipelineEncoder<'_> {
    type BufferPiece = crate::BufferPiece;

    fn dispatch(&mut self, groups: [u32; 3]) {
        self.commands.push(super::Command::Dispatch(groups));
    }

    fn dispatch_indirect(&mut self, indirect_buf: crate::BufferPiece) {
        self.commands.push(super::Command::DispatchIndirect {
            indirect_buf: indirect_buf.into(),
        });
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderEncoder for super::PipelineEncoder<'_> {
    fn set_scissor_rect(&mut self, rect: &crate::ScissorRect) {
        self.commands.push(super::Command::SetScissor(rect.clone()));
    }

    fn set_viewport(&mut self, viewport: &crate::Viewport) {
        self.commands
            .push(super::Command::SetViewport(viewport.clone()));
    }

    fn set_stencil_reference(&mut self, reference: u32) {
        unimplemented!()
    }
}

#[hidden_trait::expose]
impl crate::traits::RenderPipelineEncoder for super::PipelineEncoder<'_> {
    type BufferPiece = crate::BufferPiece;

    fn bind_vertex(&mut self, index: u32, vertex_buf: crate::BufferPiece) {
        assert_eq!(index, 0);
        self.commands.push(super::Command::BindVertex {
            buffer: vertex_buf.buffer.raw,
        });
        for info in self.vertex_attributes.iter() {
            self.commands.push(super::Command::SetVertexAttribute {
                index: info.location,
                format: info.attrib.format,
                offset: (vertex_buf.offset + info.attrib.offset as u64)
                    .try_into()
                    .unwrap(),
                stride: info.stride,
                instanced: info.instanced,
            });
        }
    }

    fn draw(
        &mut self,
        start_vertex: u32,
        vertex_count: u32,
        start_instance: u32,
        instance_count: u32,
    ) {
        assert_eq!(start_instance, 0);
        self.commands.push(super::Command::Draw {
            topology: self.topology,
            start_vertex,
            vertex_count,
            instance_count,
        });
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
        assert_eq!(start_instance, 0);
        self.commands.push(super::Command::DrawIndexed {
            topology: self.topology,
            index_buf: index_buf.into(),
            index_type: map_index_type(index_type),
            index_count,
            base_vertex,
            instance_count,
        });
    }

    fn draw_indirect(&mut self, _indirect_buf: crate::BufferPiece) {
        unimplemented!()
    }

    fn draw_indexed_indirect(
        &mut self,
        _index_buf: crate::BufferPiece,
        _index_type: crate::IndexType,
        _indirect_buf: crate::BufferPiece,
    ) {
        unimplemented!()
    }
}

impl crate::VertexFormat {
    fn describe(&self) -> (i32, u32) {
        match *self {
            Self::F32 => (1, glow::FLOAT),
            Self::F32Vec2 => (2, glow::FLOAT),
            Self::F32Vec3 => (3, glow::FLOAT),
            Self::F32Vec4 => (4, glow::FLOAT),
            Self::U32 => (1, glow::UNSIGNED_INT),
            Self::U32Vec2 => (2, glow::UNSIGNED_INT),
            Self::U32Vec3 => (3, glow::UNSIGNED_INT),
            Self::U32Vec4 => (4, glow::UNSIGNED_INT),
            Self::I32 => (1, glow::INT),
            Self::I32Vec2 => (2, glow::INT),
            Self::I32Vec3 => (3, glow::INT),
            Self::I32Vec4 => (4, glow::INT),
        }
    }
}

impl crate::BlendFactor {
    fn to_gles(self) -> u32 {
        match self {
            Self::Zero => glow::ZERO,
            Self::One => glow::ONE,
            Self::Src => glow::SRC_COLOR,
            Self::OneMinusSrc => glow::ONE_MINUS_SRC_COLOR,
            Self::SrcAlpha => glow::SRC_ALPHA,
            Self::OneMinusSrcAlpha => glow::ONE_MINUS_SRC_ALPHA,
            Self::Dst => glow::DST_COLOR,
            Self::OneMinusDst => glow::ONE_MINUS_DST_COLOR,
            Self::DstAlpha => glow::DST_ALPHA,
            Self::OneMinusDstAlpha => glow::ONE_MINUS_DST_ALPHA,
            Self::SrcAlphaSaturated => glow::SRC_ALPHA_SATURATE,
            Self::Constant => glow::CONSTANT_ALPHA,
            Self::OneMinusConstant => glow::ONE_MINUS_CONSTANT_ALPHA,
        }
    }
}

impl crate::BlendOperation {
    fn to_gles(self) -> u32 {
        match self {
            Self::Add => glow::FUNC_ADD,
            Self::Subtract => glow::FUNC_SUBTRACT,
            Self::ReverseSubtract => glow::FUNC_REVERSE_SUBTRACT,
            Self::Min => glow::MIN,
            Self::Max => glow::MAX,
        }
    }
}

const CUBEMAP_FACES: [u32; 6] = [
    glow::TEXTURE_CUBE_MAP_POSITIVE_X,
    glow::TEXTURE_CUBE_MAP_NEGATIVE_X,
    glow::TEXTURE_CUBE_MAP_POSITIVE_Y,
    glow::TEXTURE_CUBE_MAP_NEGATIVE_Y,
    glow::TEXTURE_CUBE_MAP_POSITIVE_Z,
    glow::TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

impl super::Command {
    pub(super) unsafe fn execute(&self, gl: &glow::Context, ec: &super::ExecutionContext) {
        use glow::HasContext as _;
        match *self {
            Self::Draw {
                topology,
                start_vertex,
                vertex_count,
                instance_count,
            } => {
                if instance_count == 1 {
                    gl.draw_arrays(topology, start_vertex as i32, vertex_count as i32);
                } else {
                    gl.draw_arrays_instanced(
                        topology,
                        start_vertex as i32,
                        vertex_count as i32,
                        instance_count as i32,
                    );
                }
            }
            Self::DrawIndexed {
                topology,
                ref index_buf,
                index_type,
                index_count,
                base_vertex,
                instance_count,
            } => {
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buf.raw));
                match (base_vertex, instance_count) {
                    (0, 1) => gl.draw_elements(
                        topology,
                        index_count as i32,
                        index_type,
                        index_buf.offset as i32,
                    ),
                    (0, _) => gl.draw_elements_instanced(
                        topology,
                        index_count as i32,
                        index_type,
                        index_buf.offset as i32,
                        instance_count as i32,
                    ),
                    (_, 1) => gl.draw_elements_base_vertex(
                        topology,
                        index_count as i32,
                        index_type,
                        index_buf.offset as i32,
                        base_vertex,
                    ),
                    (_, _) => gl.draw_elements_instanced_base_vertex(
                        topology,
                        index_count as _,
                        index_type,
                        index_buf.offset as i32,
                        instance_count as i32,
                        base_vertex,
                    ),
                };
            }
            Self::DrawIndirect {
                topology,
                ref indirect_buf,
            } => {
                gl.bind_buffer(glow::DRAW_INDIRECT_BUFFER, Some(indirect_buf.raw));
                gl.draw_arrays_indirect_offset(topology, indirect_buf.offset as i32);
            }
            Self::DrawIndexedIndirect {
                topology,
                raw_index_buf,
                index_type,
                ref indirect_buf,
            } => {
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(raw_index_buf));
                gl.bind_buffer(glow::DRAW_INDIRECT_BUFFER, Some(indirect_buf.raw));
                gl.draw_elements_indirect_offset(topology, index_type, indirect_buf.offset as i32);
            }
            Self::Dispatch(groups) => {
                gl.dispatch_compute(groups[0], groups[1], groups[2]);
            }
            Self::DispatchIndirect { ref indirect_buf } => {
                gl.bind_buffer(glow::DISPATCH_INDIRECT_BUFFER, Some(indirect_buf.raw));
                gl.dispatch_compute_indirect(indirect_buf.offset as i32);
            }
            Self::FillBuffer {
                dst: ref _dst,
                size: _size,
                value: _value,
            } => unimplemented!(),
            Self::CopyBufferToBuffer {
                ref src,
                ref dst,
                size,
            } => {
                gl.bind_buffer(glow::COPY_READ_BUFFER, Some(src.raw));
                gl.bind_buffer(glow::COPY_WRITE_BUFFER, Some(dst.raw));
                gl.copy_buffer_sub_data(
                    glow::COPY_READ_BUFFER,
                    glow::COPY_WRITE_BUFFER,
                    src.offset as _,
                    dst.offset as _,
                    size as _,
                );
            }
            Self::CopyTextureToTexture {
                ref src,
                ref dst,
                ref size,
            } => {
                gl.bind_texture(dst.target, Some(dst.raw));
                gl.framebuffer_texture_2d(
                    glow::READ_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    src.target,
                    Some(src.raw),
                    src.mip_level as i32,
                );
                gl.bind_texture(dst.target, Some(dst.raw));
                gl.copy_tex_sub_image_2d(
                    dst.target,
                    dst.mip_level as i32,
                    dst.origin[0] as i32,
                    dst.origin[1] as i32,
                    src.origin[0] as i32,
                    src.origin[1] as i32,
                    size.width as i32,
                    size.height as i32,
                );
                gl.framebuffer_renderbuffer(
                    glow::READ_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::RENDERBUFFER,
                    None,
                );
            }
            Self::CopyBufferToTexture {
                ref src,
                ref dst,
                bytes_per_row,
                ref size,
            } => {
                let format_desc = super::describe_texture_format(dst.format);
                let block_info = dst.format.block_info();
                let row_texels =
                    bytes_per_row / block_info.size as u32 * block_info.dimensions.0 as u32;
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, row_texels as i32);
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(src.raw));
                gl.bind_texture(dst.target, Some(dst.raw));
                let unpack_data = glow::PixelUnpackData::BufferOffset(src.offset as u32);
                match dst.target {
                    glow::TEXTURE_3D => gl.tex_sub_image_3d(
                        dst.target,
                        dst.mip_level as i32,
                        dst.origin[0] as i32,
                        dst.origin[1] as i32,
                        dst.origin[2] as i32,
                        size.width as i32,
                        size.height as i32,
                        size.depth as i32,
                        format_desc.external,
                        format_desc.data_type,
                        unpack_data,
                    ),
                    glow::TEXTURE_2D_ARRAY => gl.tex_sub_image_3d(
                        dst.target,
                        dst.mip_level as i32,
                        dst.origin[0] as i32,
                        dst.origin[1] as i32,
                        dst.origin[2] as i32,
                        size.width as i32,
                        size.height as i32,
                        size.depth as i32,
                        format_desc.external,
                        format_desc.data_type,
                        unpack_data,
                    ),
                    glow::TEXTURE_2D => gl.tex_sub_image_2d(
                        dst.target,
                        dst.mip_level as i32,
                        dst.origin[0] as i32,
                        dst.origin[1] as i32,
                        size.width as i32,
                        size.height as i32,
                        format_desc.external,
                        format_desc.data_type,
                        unpack_data,
                    ),
                    glow::TEXTURE_CUBE_MAP => gl.tex_sub_image_2d(
                        CUBEMAP_FACES[dst.array_layer as usize],
                        dst.mip_level as i32,
                        dst.origin[0] as i32,
                        dst.origin[1] as i32,
                        size.width as i32,
                        size.height as i32,
                        format_desc.external,
                        format_desc.data_type,
                        unpack_data,
                    ),
                    //Note: not sure if this is correct!
                    glow::TEXTURE_CUBE_MAP_ARRAY => gl.tex_sub_image_3d(
                        dst.target,
                        dst.mip_level as i32,
                        dst.origin[0] as i32,
                        dst.origin[1] as i32,
                        dst.origin[2] as i32,
                        size.width as i32,
                        size.height as i32,
                        size.depth as i32,
                        format_desc.external,
                        format_desc.data_type,
                        unpack_data,
                    ),
                    _ => unreachable!(),
                }
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            }
            Self::CopyTextureToBuffer {
                ref src,
                ref dst,
                bytes_per_row,
                ref size,
            } => unimplemented!(),
            Self::ResetFramebuffer => {
                for &attachment in COLOR_ATTACHMENTS.iter() {
                    gl.framebuffer_renderbuffer(
                        glow::DRAW_FRAMEBUFFER,
                        attachment,
                        glow::RENDERBUFFER,
                        None,
                    );
                }
                gl.framebuffer_renderbuffer(
                    glow::DRAW_FRAMEBUFFER,
                    glow::DEPTH_STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    None,
                );
            }

            Self::BlitFramebuffer { from, to } => {
                /*
                    TODO: Framebuffers could be re-used instead of being created on the fly.
                          Currently deleted down below
                */
                let framebuf_from = gl.create_framebuffer().unwrap();
                let framebuf_to = gl.create_framebuffer().unwrap();

                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuf_from));
                match from.inner {
                    super::TextureInner::Renderbuffer { raw } => {
                        gl.framebuffer_renderbuffer(
                            glow::READ_FRAMEBUFFER,
                            glow::COLOR_ATTACHMENT0, // NOTE: Assuming color attachment
                            glow::RENDERBUFFER,
                            Some(raw),
                        );
                    }
                    super::TextureInner::Texture { raw, target } => {
                        gl.framebuffer_texture_2d(
                            glow::READ_FRAMEBUFFER,
                            glow::COLOR_ATTACHMENT0,
                            target,
                            Some(raw),
                            0,
                        );
                    }
                }

                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(framebuf_to));
                match to.inner {
                    super::TextureInner::Renderbuffer { raw } => {
                        gl.framebuffer_renderbuffer(
                            glow::DRAW_FRAMEBUFFER,
                            glow::COLOR_ATTACHMENT0, // NOTE: Assuming color attachment
                            glow::RENDERBUFFER,
                            Some(raw),
                        );
                    }
                    super::TextureInner::Texture { raw, target } => {
                        gl.framebuffer_texture_2d(
                            glow::DRAW_FRAMEBUFFER,
                            glow::COLOR_ATTACHMENT0,
                            target,
                            Some(raw),
                            0,
                        );
                    }
                }

                debug_assert_eq!(
                    gl.check_framebuffer_status(glow::DRAW_FRAMEBUFFER),
                    glow::FRAMEBUFFER_COMPLETE,
                    "DRAW_FRAMEBUFFER is not complete"
                );

                gl.blit_framebuffer(
                    0,
                    0,
                    from.target_size[0] as _,
                    from.target_size[1] as _,
                    0,
                    0,
                    to.target_size[0] as _,
                    to.target_size[1] as _,
                    glow::COLOR_BUFFER_BIT, // NOTE: Assuming color
                    glow::NEAREST,
                );

                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);

                gl.delete_framebuffer(framebuf_from);
                gl.delete_framebuffer(framebuf_to);
            }

            Self::BindAttachment {
                attachment,
                ref view,
            } => match view.inner {
                super::TextureInner::Renderbuffer { raw } => {
                    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(ec.framebuf));
                    gl.bind_renderbuffer(glow::RENDERBUFFER, Some(raw));
                    gl.framebuffer_renderbuffer(
                        glow::DRAW_FRAMEBUFFER,
                        attachment,
                        glow::RENDERBUFFER,
                        Some(raw),
                    );
                }
                super::TextureInner::Texture { raw, target } => {
                    let mip_level = 0; //TODO
                    gl.framebuffer_texture_2d(
                        glow::DRAW_FRAMEBUFFER,
                        attachment,
                        target,
                        Some(raw),
                        mip_level,
                    );
                }
            },
            Self::InvalidateAttachment(attachment) => {
                gl.invalidate_framebuffer(glow::DRAW_FRAMEBUFFER, &[attachment]);
            }
            Self::SetDrawColorBuffers(count) => {
                gl.draw_buffers(&COLOR_ATTACHMENTS[..count as usize]);
            }
            Self::SetAllColorTargets(blend, write_mask) => {
                if let Some(blend_state) = blend {
                    gl.enable(glow::BLEND);
                    gl.blend_func_separate(
                        blend_state.color.src_factor.to_gles(),
                        blend_state.color.dst_factor.to_gles(),
                        blend_state.alpha.src_factor.to_gles(),
                        blend_state.alpha.dst_factor.to_gles(),
                    );
                    gl.blend_equation(blend_state.color.operation.to_gles());
                } else {
                    gl.disable(glow::BLEND);
                }
                gl.color_mask(
                    write_mask.contains(crate::ColorWrites::RED),
                    write_mask.contains(crate::ColorWrites::GREEN),
                    write_mask.contains(crate::ColorWrites::BLUE),
                    write_mask.contains(crate::ColorWrites::ALPHA),
                );
            }
            Self::SetSingleColorTarget(i, blend, write_mask) => {
                if let Some(blend_state) = blend {
                    gl.enable_draw_buffer(glow::BLEND, i);
                    gl.blend_func_separate_draw_buffer(
                        i,
                        blend_state.color.src_factor.to_gles(),
                        blend_state.color.dst_factor.to_gles(),
                        blend_state.alpha.src_factor.to_gles(),
                        blend_state.alpha.dst_factor.to_gles(),
                    );
                    gl.blend_equation_draw_buffer(i, blend_state.color.operation.to_gles());
                } else {
                    gl.disable_draw_buffer(glow::BLEND, i);
                }
                gl.color_mask_draw_buffer(
                    i,
                    write_mask.contains(crate::ColorWrites::RED),
                    write_mask.contains(crate::ColorWrites::GREEN),
                    write_mask.contains(crate::ColorWrites::BLUE),
                    write_mask.contains(crate::ColorWrites::ALPHA),
                );
            }
            Self::ClearColor {
                draw_buffer,
                color,
                ty,
            } => match ty {
                super::ColorType::Float => {
                    gl.clear_buffer_f32_slice(
                        glow::COLOR,
                        draw_buffer,
                        &match color {
                            crate::TextureColor::TransparentBlack => [0.0; 4],
                            crate::TextureColor::OpaqueBlack => [0.0, 0.0, 0.0, 1.0],
                            crate::TextureColor::White => [1.0; 4],
                        },
                    );
                }
                super::ColorType::Uint => {
                    gl.clear_buffer_u32_slice(
                        glow::COLOR,
                        draw_buffer,
                        &match color {
                            crate::TextureColor::TransparentBlack => [0; 4],
                            crate::TextureColor::OpaqueBlack => [0, 0, 0, !0],
                            crate::TextureColor::White => [!0; 4],
                        },
                    );
                }
                super::ColorType::Sint => {
                    gl.clear_buffer_i32_slice(
                        glow::COLOR,
                        draw_buffer,
                        &match color {
                            crate::TextureColor::TransparentBlack => [0; 4],
                            crate::TextureColor::OpaqueBlack => [0, 0, 0, !0],
                            crate::TextureColor::White => [!0; 4],
                        },
                    );
                }
            },
            Self::ClearDepthStencil { depth, stencil } => match (depth, stencil) {
                (Some(d), Some(s)) => {
                    gl.clear_buffer_depth_stencil(glow::DEPTH_STENCIL, 0, d, s as i32)
                }
                (Some(d), None) => gl.clear_buffer_f32_slice(glow::DEPTH, 0, &[d]),
                (None, Some(s)) => gl.clear_buffer_i32_slice(glow::STENCIL, 0, &[s as i32]),
                (None, None) => (),
            },
            Self::Barrier => unimplemented!(),
            Self::SetViewport(ref vp) => {
                gl.viewport(vp.x as i32, vp.y as i32, vp.w as i32, vp.h as i32);
                gl.depth_range_f32(vp.depth.start, vp.depth.end);
            }
            Self::SetScissor(ref rect) => {
                gl.scissor(rect.x, rect.y, rect.w as i32, rect.h as i32);
            }
            Self::SetStencilFunc {
                face,
                function,
                reference,
                read_mask,
            } => unimplemented!(),
            Self::SetStencilOps {
                face,
                write_mask,
                //ops: crate::StencilOps,
            } => unimplemented!(),
            //SetDepth(DepthState),
            //SetDepthBias(wgt::DepthBiasState),
            //ConfigureDepthStencil(crate::FormatAspects),
            Self::SetProgram(raw_program) => {
                gl.use_program(Some(raw_program));
            }
            Self::UnsetProgram => {
                gl.use_program(None);
            }
            //SetPrimitive(PrimitiveState),
            Self::SetBlendConstant([r, g, b, a]) => gl.blend_color(r, g, b, a),
            Self::SetColorTarget {
                draw_buffer_index,
                //desc: ColorTargetDesc,
            } => unimplemented!(),
            Self::BindUniform { slot, offset, size } => {
                gl.bind_buffer_range(
                    glow::UNIFORM_BUFFER,
                    slot,
                    Some(ec.plain_buffer),
                    offset as i32,
                    size as i32,
                );
            }
            Self::BindVertex { buffer } => {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            }
            Self::SetVertexAttribute {
                index,
                format,
                offset,
                stride,
                instanced,
            } => {
                let (data_size, data_type) = format.describe();
                match data_type {
                    glow::FLOAT => gl.vertex_attrib_pointer_f32(
                        index, data_size, data_type, false, stride, offset,
                    ),
                    glow::INT | glow::UNSIGNED_INT => {
                        gl.vertex_attrib_pointer_i32(index, data_size, data_type, stride, offset)
                    }
                    _ => unreachable!(),
                }
                gl.vertex_attrib_divisor(index, if instanced { 1 } else { 0 });
                gl.enable_vertex_attrib_array(index);
            }
            Self::DisableVertexAttributes { count } => {
                for index in 0..count {
                    gl.disable_vertex_attrib_array(index);
                }
            }
            Self::BindBuffer {
                target,
                slot,
                ref buffer,
                size,
            } => {
                gl.bind_buffer_range(
                    target,
                    slot,
                    Some(buffer.raw),
                    buffer.offset as i32,
                    size as i32,
                );
            }
            Self::BindSampler { slot, sampler } => {
                gl.bind_sampler(slot, Some(sampler));
            }
            Self::BindTexture {
                slot,
                texture,
                target,
            } => {
                gl.active_texture(glow::TEXTURE0 + slot);
                gl.bind_texture(target, Some(texture));
            }
            Self::BindImage { slot, ref binding } => unimplemented!(),
            Self::ResetAllSamplers => {
                gl.active_texture(glow::TEXTURE0);
                for slot in 0..4 {
                    gl.bind_sampler(slot, None);
                }
            }
            Self::QueryCounter { query } => {
                gl.query_counter(query, glow::TIMESTAMP);
            }
            Self::PushScope { ref name_range } => {
                let name = str::from_utf8(&ec.string_data[name_range.clone()]).unwrap();
                gl.push_debug_group(glow::DEBUG_SOURCE_APPLICATION, super::DEBUG_ID, name);
            }
            Self::PopScope => {
                gl.pop_debug_group();
            }
        }
    }
}

fn map_index_type(ty: crate::IndexType) -> u32 {
    match ty {
        crate::IndexType::U16 => glow::UNSIGNED_SHORT,
        crate::IndexType::U32 => glow::UNSIGNED_INT,
    }
}

fn map_primitive_topology(topology: crate::PrimitiveTopology) -> u32 {
    use crate::PrimitiveTopology as Pt;
    match topology {
        Pt::PointList => glow::POINTS,
        Pt::LineList => glow::LINES,
        Pt::LineStrip => glow::LINE_STRIP,
        Pt::TriangleList => glow::TRIANGLES,
        Pt::TriangleStrip => glow::TRIANGLE_STRIP,
    }
}
