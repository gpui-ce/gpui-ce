//! Guards the shared shader boundary used by the WGPU, Metal, and DirectX renderers.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("gpui crate must live at <workspace>/crates/gpui")
        .to_path_buf()
}

fn collect_renderer_shader_sources(root: &Path, sources: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", root.display()));

    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to inspect an entry under {}: {error}",
                root.display()
            )
        });
        let path = entry.path();
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("failed to inspect {}: {error}", path.display()));

        if file_type.is_dir() {
            collect_renderer_shader_sources(&path, sources);
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("wgsl" | "metal" | "hlsl")
        ) {
            sources.push(path);
        }
    }
}

fn relative_paths(root: &Path, paths: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    let mut relative: Vec<_> = paths
        .into_iter()
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
        })
        .collect();
    relative.sort();
    relative
}

#[test]
fn renderer_backends_share_rust_authored_shaders() {
    let root = workspace_root();

    let native_renderer_modules = [
        "crates/gpui_macos/src/metal_renderer.rs",
        "crates/gpui_windows/src/directx_renderer.rs",
    ];
    for relative in native_renderer_modules {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assert!(
            source.contains("NATIVE_SHADERS"),
            "{} must consume generated shared shader artifacts",
            path.display()
        );
    }

    let mut platform_shader_sources = Vec::new();
    for relative in ["crates/gpui_macos/src", "crates/gpui_windows/src"] {
        let path = root.join(relative);
        if path.exists() {
            collect_renderer_shader_sources(&path, &mut platform_shader_sources);
        }
    }
    assert!(
        platform_shader_sources.is_empty(),
        "platform-local renderer shader source bypasses shared generation: {}",
        relative_paths(&root, platform_shader_sources).join(", ")
    );

    let rust_shader_module = root.join("crates/gpui_render/src/shaders/mod.rs");
    assert!(
        rust_shader_module.is_file(),
        "renderers must keep their typed Rust shader module in gpui_render"
    );

    let mut standalone_shader_sources = Vec::new();
    for relative in ["crates/gpui_render/src", "crates/gpui_wgpu/src"] {
        collect_renderer_shader_sources(&root.join(relative), &mut standalone_shader_sources);
    }
    assert!(
        standalone_shader_sources.is_empty(),
        "standalone shader sources bypass typed generation: {}",
        relative_paths(&root, standalone_shader_sources).join(", ")
    );
}

#[test]
fn directx_registers_and_instance_views_follow_generated_shader_contracts() {
    let path = workspace_root().join("crates/gpui_windows/src/directx_renderer.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));

    for binding in [
        "shader_interface::DATA_BUFFER_BINDING",
        "shader_interface::PRIMARY_TEXTURE_BINDING",
        "shader_interface::PRIMARY_SAMPLER_BINDING",
        "shader_interface::SURFACE_SAMPLER_BINDING",
    ] {
        assert!(
            source.contains(binding),
            "{} must derive native register slots from {binding}",
            path.display()
        );
    }
    assert!(
        source.contains("self.pipelines.surfaces.params_buffer"),
        "{} must bind the surface pipeline's generated uniform buffer",
        path.display()
    );
    assert!(
        source.contains("PSSetSamplers(SURFACE_SAMPLER_REGISTER"),
        "{} must bind the surface sampler at its generated slot",
        path.display()
    );
    assert!(
        !source.contains("create_buffer_view_range"),
        "{} must reuse each pipeline's whole-buffer SRV and select batches with first-instance",
        path.display()
    );
}
