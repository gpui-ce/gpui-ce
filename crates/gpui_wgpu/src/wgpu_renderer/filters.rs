use gpui::{BackdropFilter, ScaledPixels};
use gpui_render::shaders::interface as shader_interface;
use gpui_render::{
    blur::{
        downsampled_dimension, BlurAxis, BlurKernel, FilterCompositeClip,
        FilterCompositeParameters, ScissorRectangle, GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS,
    },
    shaders::blur::BlurUniforms,
};

use super::{begin_color_render_pass, pipelines, WgpuRenderer};

pub(super) const FILTER_UNIFORMS_PER_COMPOSITE: u64 = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FrameUniformRequirements {
    pub(super) filter_count: u64,
    pub(super) surface_count: u64,
}

const _: () = assert!(std::mem::size_of::<BlurUniforms>() == 96);

impl WgpuRenderer {
    fn make_blur_bind_group(
        &self,
        uniforms: BlurUniforms,
        source: &wgpu::TextureView,
    ) -> (wgpu::BindGroup, u32) {
        let resources = self.resources();
        let uniform_offset = resources.filter_uniforms.write(&uniforms);
        (resources.blur_bind_group(source), uniform_offset)
    }

    fn run_blur_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &pipelines::WgpuRenderPipeline,
        target: &wgpu::TextureView,
        source: &wgpu::TextureView,
        uniforms: BlurUniforms,
        scissor: ScissorRectangle,
    ) {
        let (bind_group, uniform_offset) = self.make_blur_bind_group(uniforms, source);
        let resources = self.resources();
        let mut pass = begin_color_render_pass(
            encoder,
            label,
            target,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
        pass.set_pipeline(pipeline);
        pass.set_bind_group(
            shader_interface::GLOBAL_BIND_GROUP,
            &resources.globals_bind_group,
            &[],
        );
        pass.set_bind_group(
            shader_interface::DATA_BIND_GROUP,
            &bind_group,
            &[uniform_offset],
        );
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        pass.draw(0..pipeline.fixed_vertex_count(), 0..1);
    }

    pub(super) fn blur_and_composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        parameters: FilterCompositeParameters,
    ) {
        let Some(kernel) = BlurKernel::for_radius(parameters.blur_radius) else {
            return;
        };
        let full_width = self.target.width();
        let full_height = self.target.height();
        let blur_size = [
            downsampled_dimension(full_width) as f32,
            downsampled_dimension(full_height) as f32,
        ];
        let dilation = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * parameters.blur_radius;
        let scissor = ScissorRectangle::for_blurred_bounds(
            parameters.bounds,
            dilation,
            full_width,
            full_height,
        );
        if scissor.is_empty() {
            return;
        }

        let (horizontal_target, vertical_target) = {
            let resources = self.resources();
            match (
                resources.blur_ping_view.as_ref(),
                resources.blur_pong_view.as_ref(),
            ) {
                (Some(horizontal), Some(vertical)) => (horizontal.clone(), vertical.clone()),
                _ => return,
            }
        };

        self.run_blur_pass(
            encoder,
            "blur_downsample",
            &self.resources().pipelines.blur_downsample,
            &horizontal_target,
            source,
            BlurUniforms::downsample([full_width as f32, full_height as f32], blur_size),
            scissor,
        );
        self.run_blur_pass(
            encoder,
            "blur_horizontal",
            &self.resources().pipelines.blur,
            &vertical_target,
            &horizontal_target,
            BlurUniforms::gaussian(BlurAxis::Horizontal, blur_size, kernel),
            scissor,
        );
        self.run_blur_pass(
            encoder,
            "blur_vertical",
            &self.resources().pipelines.blur,
            &horizontal_target,
            &vertical_target,
            BlurUniforms::gaussian(BlurAxis::Vertical, blur_size, kernel),
            scissor,
        );

        let clips_to_bounds = matches!(parameters.clip, FilterCompositeClip::RoundedBounds);
        let composite_bounds = if clips_to_bounds {
            parameters.bounds
        } else {
            parameters.bounds.dilate(ScaledPixels(dilation))
        };
        let uniforms = BlurUniforms::composite(
            composite_bounds,
            parameters.content_mask,
            parameters.corner_radii,
            parameters.opacity,
            parameters.clip,
            blur_size,
            [full_width as f32, full_height as f32],
        );
        let (bind_group, uniform_offset) = self.make_blur_bind_group(uniforms, &horizontal_target);
        let resources = self.resources();
        let mut pass =
            begin_color_render_pass(encoder, "blur_composite", target, wgpu::LoadOp::Load);
        pass.set_pipeline(&resources.pipelines.blur_composite);
        pass.set_bind_group(
            shader_interface::GLOBAL_BIND_GROUP,
            &resources.globals_bind_group,
            &[],
        );
        pass.set_bind_group(
            shader_interface::DATA_BIND_GROUP,
            &bind_group,
            &[uniform_offset],
        );
        pass.draw(
            0..resources.pipelines.blur_composite.fixed_vertex_count(),
            0..1,
        );
    }

    pub(super) fn draw_backdrop_filter(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        filter: &BackdropFilter,
        scene_color_view: &wgpu::TextureView,
    ) {
        self.blur_and_composite(
            encoder,
            scene_color_view,
            scene_color_view,
            FilterCompositeParameters {
                bounds: filter.bounds,
                content_mask: filter.content_mask.bounds,
                corner_radii: filter.corner_radii,
                blur_radius: filter.max_blur_radius(),
                opacity: filter.opacity,
                clip: FilterCompositeClip::RoundedBounds,
            },
        );
    }

    pub(super) fn blit_to_frame(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        frame_view: &wgpu::TextureView,
    ) {
        let size = [self.target.width() as f32, self.target.height() as f32];
        let (bind_group, uniform_offset) =
            self.make_blur_bind_group(BlurUniforms::copy(size), source);
        let resources = self.resources();
        let mut pass = begin_color_render_pass(
            encoder,
            "scene_blit",
            frame_view,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
        pass.set_pipeline(&resources.pipelines.blur_downsample);
        pass.set_bind_group(
            shader_interface::GLOBAL_BIND_GROUP,
            &resources.globals_bind_group,
            &[],
        );
        pass.set_bind_group(
            shader_interface::DATA_BIND_GROUP,
            &bind_group,
            &[uniform_offset],
        );
        pass.draw(
            0..resources.pipelines.blur_downsample.fixed_vertex_count(),
            0..1,
        );
    }
}
