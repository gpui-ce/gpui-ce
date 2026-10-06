//! Link paint and custom pipeline programs with GPUI's renderer functions.
//!
//! Shader quads reuse the ordinary quad pipeline, replacing [`paint_hook`]
//! with [`paint_glue`]. wgsl-rs emits each source identity once: importing an
//! empty source with the hook's identity first suppresses its stub. The glue
//! calls the program's `paint_program`, appended after the generated module.

use std::sync::OnceLock;

use wgsl_rs::{Source, ir};

use crate::shaders::{paint_glue, paint_hook, pipeline_glue, quad};

fn no_items() -> ir::Module {
    ir::Module {
        name: "shadow",
        items: vec![],
        attrs: vec![],
    }
}

/// A source with `source`'s identity and no items.
const fn shadow(source: &'static Source) -> Source {
    Source {
        id: source.id,
        name: source.name,
        imports: &[],
        ir_constructor: no_items,
        templates: &[],
        instantiations: &[],
        module_type_params: &[],
    }
}

static HOOK_SHADOW: Source = shadow(&paint_hook::WGSL_SOURCE);

static SHADER_QUAD: Source = Source {
    id: u64::from_le_bytes(*b"gpuishdq"),
    name: "shader_quad",
    imports: &[&HOOK_SHADOW, &quad::WGSL_SOURCE, &paint_glue::WGSL_SOURCE],
    ir_constructor: no_items,
    templates: &[],
    instantiations: &[],
    module_type_params: &[],
};

/// Bindings a shader quad pipeline adds in group 2.
pub mod bindings {
    /// `array<vec4<f32>>` of parameter blocks, read-only storage.
    pub const PARAMS: u32 = 0;
    /// The backdrop snapshot, a filterable 2D float texture.
    pub const BACKDROP: u32 = 1;
    /// A filtering sampler for the backdrop.
    pub const BACKDROP_SAMPLER: u32 = 2;
    /// The bind group for parameters and optional backdrop bindings.
    pub const GROUP: u32 = 2;
}

/// The complete WGSL module for quads painted by `program`: the quad pipeline's
/// vertex and fragment entry points, with shader backgrounds running the program.
pub fn shader_quad_module(program: &gpui::shader::Program) -> String {
    static BASE: OnceLock<String> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        SHADER_QUAD
            .wgsl_source()
            .expect("the shader quad sources are concrete")
    });
    // Paint derivatives run behind the quad's per-pixel clip, as in any UI shader.
    format!(
        "diagnostic(off, derivative_uniformity);\n{base}\n{}",
        program.source()
    )
}

/// The complete WGSL module for a custom pipeline: its generated entry points
/// and source, linked against the pipeline glue.
pub fn pipeline_module(pipeline: &gpui::shader::Pipeline) -> String {
    static BASE: OnceLock<String> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        pipeline_glue::WGSL_SOURCE
            .wgsl_source()
            .expect("the pipeline glue is concrete")
    });
    format!("{base}\n{}", pipeline.source())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::shader::{color, paint, rgba};

    fn validate(source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
    }

    #[test]
    fn linked_programs_replace_the_hook_and_validate() {
        let ramp = paint(|px| {
            let edge = (px.uv().x() - 0.5).fwidth();
            rgba(px.uv().x(), edge, 0.0, 1.0).over(color(gpui::red()))
        });
        let glass = paint(|px| px.backdrop(px.uv().yx()));
        for paint in [ramp, glass] {
            let module = shader_quad_module(&paint.compile().unwrap().program);
            assert_eq!(module.matches("fn shader_paint(").count(), 1);
            assert!(module.contains("fragment_quad"));
            validate(&module);
        }
    }

    #[test]
    fn linked_pipelines_validate() {
        let pipeline = gpui::shader::Pipeline::wgsl(
            "struct Varyings { position: vec2<f32>, local: vec2<f32> }
            fn vertex(index: u32, center: vec2<f32>) -> Varyings {
                let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
                return Varyings(center + corner, corner);
            }
            fn fragment(in: Varyings) -> vec4<f32> { return vec4<f32>(in.local, 0.0, 1.0); }",
        )
        .unwrap();
        validate(&pipeline_module(&pipeline));
    }
}
