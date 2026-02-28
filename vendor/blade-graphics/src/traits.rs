use std::{fmt::Debug, hash::Hash};

pub trait ResourceDevice {
    type Buffer: Send + Sync + Clone + Copy + Debug + Hash + PartialEq;
    type Texture: Send + Sync + Clone + Copy + Debug + Hash + PartialEq;
    type TextureView: Send + Sync + Clone + Copy + Debug + Hash + PartialEq;
    type Sampler: Send + Sync + Clone + Copy + Debug + Hash + PartialEq;
    type AccelerationStructure: Send + Sync + Clone + Copy + Debug + Hash + PartialEq;

    fn create_buffer(&self, desc: super::BufferDesc) -> Self::Buffer;
    fn sync_buffer(&self, buffer: Self::Buffer);
    fn destroy_buffer(&self, buffer: Self::Buffer);
    fn create_texture(&self, desc: super::TextureDesc) -> Self::Texture;
    fn destroy_texture(&self, texture: Self::Texture);
    fn create_texture_view(
        &self,
        texture: Self::Texture,
        desc: super::TextureViewDesc,
    ) -> Self::TextureView;
    fn destroy_texture_view(&self, view: Self::TextureView);
    fn create_sampler(&self, desc: super::SamplerDesc) -> Self::Sampler;
    fn destroy_sampler(&self, sampler: Self::Sampler);
    fn create_acceleration_structure(
        &self,
        desc: super::AccelerationStructureDesc,
    ) -> Self::AccelerationStructure;
    fn destroy_acceleration_structure(&self, acceleration_structure: Self::AccelerationStructure);
}

pub trait ShaderDevice {
    type ComputePipeline: Send + Sync;
    type RenderPipeline: Send + Sync;

    fn create_compute_pipeline(&self, desc: super::ComputePipelineDesc) -> Self::ComputePipeline;
    fn destroy_compute_pipeline(&self, pipeline: &mut Self::ComputePipeline);
    fn create_render_pipeline(&self, desc: super::RenderPipelineDesc) -> Self::RenderPipeline;
    fn destroy_render_pipeline(&self, pipeline: &mut Self::RenderPipeline);
}

pub trait CommandDevice {
    type CommandEncoder;
    type SyncPoint: Clone + Debug;

    fn create_command_encoder(&self, desc: super::CommandEncoderDesc) -> Self::CommandEncoder;
    fn destroy_command_encoder(&self, encoder: &mut Self::CommandEncoder);
    fn submit(&self, encoder: &mut Self::CommandEncoder) -> Self::SyncPoint;
    fn wait_for(&self, sp: &Self::SyncPoint, timeout_ms: u32) -> bool;
}

pub trait CommandEncoder {
    type Texture: Send + Sync + Clone + Copy + Debug;
    type Frame: Send + Sync + Debug;
    fn start(&mut self);
    fn init_texture(&mut self, texture: Self::Texture);
    fn present(&mut self, frame: Self::Frame);
    fn timings(&self) -> &super::Timings;
}

pub trait TransferEncoder {
    type BufferPiece: Send + Sync + Clone + Copy + Debug;
    type TexturePiece: Send + Sync + Clone + Copy + Debug;

    fn fill_buffer(&mut self, dst: Self::BufferPiece, size: u64, value: u8);
    fn copy_buffer_to_buffer(&mut self, src: Self::BufferPiece, dst: Self::BufferPiece, size: u64);
    fn copy_texture_to_texture(
        &mut self,
        src: Self::TexturePiece,
        dst: Self::TexturePiece,
        size: super::Extent,
    );

    fn copy_buffer_to_texture(
        &mut self,
        src: Self::BufferPiece,
        bytes_per_row: u32,
        dst: Self::TexturePiece,
        size: super::Extent,
    );

    fn copy_texture_to_buffer(
        &mut self,
        src: Self::TexturePiece,
        dst: Self::BufferPiece,
        bytes_per_row: u32,
        size: super::Extent,
    );
}

pub trait AccelerationStructureEncoder {
    type AccelerationStructure: Send + Sync + Clone + Debug;
    type AccelerationStructureMesh: Send + Sync + Clone + Debug;
    type BufferPiece: Send + Sync + Clone + Copy + Debug;

    fn build_bottom_level(
        &mut self,
        acceleration_structure: Self::AccelerationStructure,
        meshes: &[Self::AccelerationStructureMesh],
        scratch_data: Self::BufferPiece,
    );

    fn build_top_level(
        &mut self,
        acceleration_structure: Self::AccelerationStructure,
        bottom_level: &[Self::AccelerationStructure],
        instance_count: u32,
        instance_data: Self::BufferPiece,
        scratch_data: Self::BufferPiece,
    );
}

pub trait RenderEncoder {
    fn set_scissor_rect(&mut self, rect: &super::ScissorRect);
    fn set_viewport(&mut self, viewport: &super::Viewport);
    fn set_stencil_reference(&mut self, reference: u32);
}

pub trait PipelineEncoder {
    fn bind<D: super::ShaderData>(&mut self, group: u32, data: &D);
}

pub trait ComputePipelineEncoder: PipelineEncoder {
    type BufferPiece: Send + Sync + Clone + Copy + Debug;

    fn dispatch(&mut self, groups: [u32; 3]);
    fn dispatch_indirect(&mut self, indirect_buf: Self::BufferPiece);
}

pub trait RenderPipelineEncoder: PipelineEncoder + RenderEncoder {
    type BufferPiece: Send + Sync + Clone + Copy + Debug;

    //Note: does this need to be available outside of the pipeline?
    fn bind_vertex(&mut self, index: u32, vertex_buf: Self::BufferPiece);
    fn draw(
        &mut self,
        first_vertex: u32,
        vertex_count: u32,
        first_instance: u32,
        instance_count: u32,
    );
    fn draw_indexed(
        &mut self,
        index_buf: Self::BufferPiece,
        index_type: super::IndexType,
        index_count: u32,
        base_vertex: i32,
        start_instance: u32,
        instance_count: u32,
    );
    fn draw_indirect(&mut self, indirect_buf: Self::BufferPiece);
    fn draw_indexed_indirect(
        &mut self,
        index_buf: Self::BufferPiece,
        index_type: crate::IndexType,
        indirect_buf: Self::BufferPiece,
    );
}
