use gpui::{Bounds, Corners, ScaledPixels, point, size};
use gpui_render::shaders::interface as shader_interface;
use gpui_render::{
    blur::{
        BlurPass, BlurPlan, FilterCompositeClip, FilterCompositeParameters, FilterSource,
        GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS, ScissorRectangle, visible_composite_region,
    },
    shaders::blur::BlurUniforms,
};

use super::{WgpuRenderer, begin_color_render_pass, pipelines};

pub(super) const FILTER_UNIFORMS_PER_COMPOSITE: u64 = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FrameUniformRequirements {
    pub(super) filter_count: u64,
    pub(super) surface_count: u64,
}

const _: () = assert!(std::mem::size_of::<BlurUniforms>() == 304);

impl WgpuRenderer {
    fn draw_filter_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &pipelines::WgpuRenderPipeline,
        target: &wgpu::TextureView,
        source: &wgpu::TextureView,
        uniforms: BlurUniforms,
        load: wgpu::LoadOp<wgpu::Color>,
        scissor: Option<ScissorRectangle>,
    ) {
        let resources = self.resources();
        let uniform_offset = resources.filter_uniforms.write(&uniforms);
        let bind_group = resources.blur_bind_group(source);
        let mut pass = begin_color_render_pass(encoder, label, target, load);
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

        if let Some(scissor) = scissor {
            pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        }

        pass.draw(0..pipeline.fixed_vertex_count(), 0..1);
    }

    pub(super) fn blur_and_composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        parameters: FilterCompositeParameters,
    ) {
        if parameters.blur_radius <= 0.0
            && matches!(parameters.clip, FilterCompositeClip::RoundedBounds)
        {
            return;
        }

        let target_size = [self.target.width(), self.target.height()];
        let Some(region) = visible_composite_region(
            composite_bounds(&parameters),
            parameters.content_mask,
            target_size,
        ) else {
            return;
        };
        let Some(plan) = BlurPlan::new(parameters.blur_radius, region, target_size) else {
            self.composite_texture(
                encoder,
                source,
                target,
                parameters,
                FilterSource::full_resolution(target_size[0], target_size[1]),
            );
            return;
        };

        let (ping, pong) = {
            let resources = self.resources();
            match (
                resources.blur_ping_view.as_ref(),
                resources.blur_pong_view.as_ref(),
            ) {
                (Some(ping), Some(pong)) => (ping.clone(), pong.clone()),
                _ => return,
            }
        };

        let resources = self.resources();
        for (label, pipeline, pass, source, target) in [
            (
                "blur_downsample",
                &resources.pipelines.blur_downsample,
                plan.downsample,
                source,
                &ping,
            ),
            (
                "blur_horizontal",
                &resources.pipelines.blur,
                plan.horizontal,
                &ping,
                &pong,
            ),
            (
                "blur_vertical",
                &resources.pipelines.blur,
                plan.vertical,
                &pong,
                &ping,
            ),
        ] {
            let BlurPass { uniforms, scissor } = pass;
            self.draw_filter_pass(
                encoder,
                label,
                pipeline,
                target,
                source,
                uniforms,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                Some(scissor),
            );
        }

        self.composite_texture(encoder, &ping, target, parameters, plan.result);
    }

    fn composite_texture(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        parameters: FilterCompositeParameters,
        filter_source: FilterSource,
    ) {
        let uniforms = BlurUniforms::composite(
            composite_bounds(&parameters),
            parameters.content_mask,
            parameters.corner_radii,
            parameters.corner_smoothing,
            parameters.opacity,
            parameters.clip,
            filter_source,
        );
        let resources = self.resources();
        let pipeline = if uniforms.corner_smoothing > 0.0 {
            &resources.pipelines.smoothed_blur_composite
        } else {
            &resources.pipelines.blur_composite
        };

        self.draw_filter_pass(
            encoder,
            "blur_composite",
            pipeline,
            target,
            source,
            uniforms,
            wgpu::LoadOp::Load,
            None,
        );
    }

    pub(super) fn snapshot_backdrop<'a>(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        sources: impl Iterator<Item = &'a wgpu::TextureView>,
    ) -> wgpu::TextureView {
        let target = self
            .resources()
            .backdrop_snapshot_view
            .as_ref()
            .expect("backdrop snapshot was prepared")
            .clone();
        let full_size = [self.target.width() as f32, self.target.height() as f32];
        let bounds = Bounds::new(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(full_size[0]), ScaledPixels(full_size[1])),
        );
        drop(begin_color_render_pass(
            encoder,
            "clear_backdrop_snapshot",
            &target,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        ));

        for source in sources {
            self.composite_texture(
                encoder,
                source,
                &target,
                FilterCompositeParameters {
                    bounds,
                    content_mask: bounds,
                    corner_radii: Corners::default(),
                    corner_smoothing: 0.0,
                    blur_radius: 0.0,
                    opacity: 1.0,
                    clip: FilterCompositeClip::ContentShape,
                },
                FilterSource::full_resolution(self.target.width(), self.target.height()),
            );
        }

        target
    }

    pub(super) fn blit_to_frame(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        frame_view: &wgpu::TextureView,
    ) {
        let size = [self.target.width() as f32, self.target.height() as f32];

        self.draw_filter_pass(
            encoder,
            "scene_blit",
            &self.resources().pipelines.blur_downsample,
            frame_view,
            source,
            BlurUniforms::copy(size),
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            None,
        );
    }
}

/// Content filters bleed past their bounds by the blur's reach; backdrop filters clip to them.
fn composite_bounds(parameters: &FilterCompositeParameters) -> Bounds<ScaledPixels> {
    match parameters.clip {
        FilterCompositeClip::RoundedBounds => parameters.bounds,
        FilterCompositeClip::ContentShape => parameters.bounds.dilate(ScaledPixels(
            GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * parameters.blur_radius,
        )),
    }
}
