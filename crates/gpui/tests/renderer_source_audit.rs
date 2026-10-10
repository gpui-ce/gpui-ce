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
        "crates/gpui_apple/src/metal_renderer.rs",
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

    let macos_root = root.join("crates/gpui_macos/src");
    assert!(
        !macos_root.join("metal_renderer.rs").exists(),
        "the extracted Apple renderer must not retain a macOS compatibility module"
    );
    let macos_lib = fs::read_to_string(macos_root.join("gpui_macos.rs"))
        .expect("failed to read the macOS crate root");
    assert!(
        !macos_lib.contains("pub mod metal_renderer"),
        "the macOS crate must not re-export the extracted renderer"
    );

    let mut platform_shader_sources = Vec::new();
    for relative in [
        "crates/gpui_apple/src",
        "crates/gpui_macos/src",
        "crates/gpui_windows/src",
    ] {
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

#[test]
fn native_renderer_fallbacks_and_intermediates_are_lazy() {
    let root = workspace_root();
    let wgpu_context_path = root.join("crates/gpui_wgpu/src/wgpu_context.rs");
    let wgpu_context = fs::read_to_string(&wgpu_context_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", wgpu_context_path.display()));
    assert!(
        wgpu_context.contains("enum NativeBackend"),
        "{} must represent one native backend as a closed type",
        wgpu_context_path.display()
    );
    assert!(
        wgpu_context.contains("impl From<NativeBackend> for wgpu::Backends")
            && wgpu_context.contains("backends: self.into()")
            && wgpu_context.contains("try_in_preference_order"),
        "{} must initialize one backend per fallback attempt",
        wgpu_context_path.display()
    );
    assert!(
        !wgpu_context.contains("Backends::VULKAN | wgpu::Backends::GL"),
        "{} must not eagerly construct the GL fallback beside Vulkan",
        wgpu_context_path.display()
    );

    let directx_path = root.join("crates/gpui_windows/src/directx_renderer.rs");
    let directx = fs::read_to_string(&directx_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directx_path.display()));
    assert!(
        directx.contains("blur: Option<BlurResources>"),
        "{} must make large filter targets optional",
        directx_path.display()
    );
    assert!(
        directx.contains("path: Option<PathResources>"),
        "{} must make large path targets optional",
        directx_path.display()
    );
    assert!(
        directx.contains("ensure_blur_resources(device, requirements.isolated_target_count)"),
        "{} must allocate filter targets from the typed scene requirements",
        directx_path.display()
    );
    assert!(
        directx.contains("ShaderModule::Blur.bytecode()")
            && directx.contains("create_vertex_shader(device, bytecode.vertex)"),
        "{} must create DirectX shaders from generated bytecode",
        directx_path.display()
    );
    assert!(
        directx.contains("surface_views: FxHashMap<usize, CachedSurfaceView>"),
        "{} must reuse capture texture views while their surfaces remain active",
        directx_path.display()
    );
}

/// The `wgpu` feature gate a Windows platform source line sits behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WgpuGate {
    /// Compiled into every build of the crate.
    Ungated,
    /// Behind `#[cfg(feature = "wgpu")]`.
    Wgpu,
    /// Behind `#[cfg(not(feature = "wgpu"))]`.
    NotWgpu,
}

const WGPU_GATE: &str = "#[cfg(feature = \"wgpu\")]";
const NOT_WGPU_GATE: &str = "#[cfg(not(feature = \"wgpu\"))]";

/// Counts the brackets a line opens and closes outside string literals.
fn bracket_delta(line: &str) -> (usize, usize) {
    let (mut opened, mut closed, mut in_string, mut escaped) = (0, 0, false, false);
    for character in line.chars() {
        match character {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => in_string = !in_string,
            '(' | '[' | '{' if !in_string => opened += 1,
            ')' | ']' | '}' if !in_string => closed += 1,
            _ => {}
        }
        if character != '\\' {
            escaped = false;
        }
    }
    (opened, closed)
}

/// Tags every non-comment line of `source` with the `wgpu` feature gate it
/// sits behind. A gate attribute covers the item, field, statement or match
/// arm that follows it: through the line that closes every bracket it
/// opened, or its first line when it opens none. Gate attribute lines
/// themselves are dropped.
fn wgpu_gated_lines(source: &str) -> Vec<(usize, &str, WgpuGate)> {
    let mut lines = Vec::new();
    let mut pending = None;
    // The gate of the item being walked, its open bracket depth, and
    // whether it has opened a bracket yet.
    let mut active: Option<(WgpuGate, i64, bool)> = None;

    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }

        if active.is_none() {
            let gate_and_rest = if let Some(rest) = trimmed.strip_prefix(WGPU_GATE) {
                Some((WgpuGate::Wgpu, rest.trim()))
            } else if let Some(rest) = trimmed.strip_prefix(NOT_WGPU_GATE) {
                Some((WgpuGate::NotWgpu, rest.trim()))
            } else {
                None
            };
            match gate_and_rest {
                Some((gate, "")) => {
                    pending = Some(gate);
                    continue;
                }
                Some((gate, rest)) => {
                    lines.push((number, rest, gate));
                    active = Some((gate, 0, false));
                }
                None => match pending {
                    Some(gate) if trimmed.starts_with("#[") => {
                        lines.push((number, line, gate));
                        continue;
                    }
                    Some(gate) => {
                        lines.push((number, line, gate));
                        active = Some((gate, 0, false));
                        pending = None;
                    }
                    None => {
                        lines.push((number, line, WgpuGate::Ungated));
                        continue;
                    }
                },
            }
        } else {
            let gate = active.map(|(gate, _, _)| gate).unwrap();
            lines.push((number, line, gate));
        }

        let (gate, depth, opened_any) = active.unwrap();
        let (opened, closed) = bracket_delta(trimmed);
        let depth = depth + opened as i64 - closed as i64;
        let opened_any = opened_any || opened > 0;
        let item_ends =
            depth <= 0 && (opened_any || trimmed.ends_with(';') || trimmed.ends_with(','));
        active = (!item_ends).then_some((gate, depth, opened_any));
    }

    lines
}

fn line_gate(lines: &[(usize, &str, WgpuGate)], needle: &str) -> Option<WgpuGate> {
    lines
        .iter()
        .find(|(_, line, _)| line.contains(needle))
        .map(|(_, _, gate)| *gate)
}

#[test]
fn windows_wgpu_renderer_is_opt_in() {
    let root = workspace_root();

    let manifest_path = root.join("crates/gpui_windows/Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", manifest_path.display()));
    let mut section = String::new();
    let mut features: Vec<(String, String)> = Vec::new();
    let mut open_feature: Option<(String, String, i64)> = None;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, body, depth)) = open_feature.as_mut() {
            body.push_str(trimmed);
            let (opened, closed) = bracket_delta(trimmed);
            *depth += opened as i64 - closed as i64;
            if *depth <= 0 {
                features.push((name.clone(), body.clone()));
                open_feature = None;
            }
            continue;
        }
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if section == "[features]" {
            let (opened, closed) = bracket_delta(value);
            let depth = opened as i64 - closed as i64;
            if depth > 0 {
                open_feature = Some((key.to_string(), value.trim().to_string(), depth));
            } else {
                features.push((key.to_string(), value.trim().to_string()));
            }
        } else if section.ends_with("dependencies]") {
            let dependency = key.split('.').next().unwrap_or(key);
            if dependency.contains("wgpu") {
                assert_eq!(
                    dependency,
                    "gpui_wgpu",
                    "{} must reach WGPU only through gpui_wgpu, never a direct wgpu dependency",
                    manifest_path.display()
                );
                assert!(
                    value.contains("optional = true"),
                    "{} must declare gpui_wgpu optional so the default build never links it",
                    manifest_path.display()
                );
            }
        }
    }
    let feature = |name: &str| {
        features
            .iter()
            .find(|(feature, _)| feature == name)
            .map(|(_, body)| body.as_str())
    };
    assert!(
        !feature("default").unwrap_or_default().contains("wgpu"),
        "{} must not enable the wgpu feature by default",
        manifest_path.display()
    );
    assert!(
        feature("wgpu").is_some_and(|body| body.contains("\"dep:gpui_wgpu\"")),
        "{} must enable gpui_wgpu from an opt-in `wgpu` feature",
        manifest_path.display()
    );
    for (name, body) in &features {
        assert!(
            name == "wgpu" || !(body.contains("dep:gpui_wgpu") || body.contains("\"gpui_wgpu/")),
            "{} feature `{name}` must not enable gpui_wgpu; only `wgpu` may (weak `gpui_wgpu?/` is fine)",
            manifest_path.display()
        );
    }

    let crate_root_path = root.join("crates/gpui_windows/src/gpui_windows.rs");
    let window_path = root.join("crates/gpui_windows/src/window.rs");
    let platform_path = root.join("crates/gpui_windows/src/platform.rs");
    let events_path = root.join("crates/gpui_windows/src/events.rs");
    let mut gated = Vec::new();
    for path in [&crate_root_path, &window_path, &platform_path, &events_path] {
        let source = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let lines: Vec<_> = wgpu_gated_lines(&source)
            .into_iter()
            .map(|(number, line, gate)| (number, line.to_string(), gate))
            .collect();
        for (number, line, gate) in &lines {
            let trimmed = line.trim();
            if trimmed.starts_with("#[") || !trimmed.to_ascii_lowercase().contains("wgpu") {
                continue;
            }
            assert_eq!(
                *gate,
                WgpuGate::Wgpu,
                "{}:{number} must keep every WGPU renderer reference behind {WGPU_GATE}: {trimmed}",
                path.display()
            );
        }
        gated.push((path, lines));
    }
    let lines_of = |wanted: &Path| {
        let (_, lines) = gated.iter().find(|(path, _)| *path == wanted).unwrap();
        lines
            .iter()
            .map(|(number, line, gate)| (*number, line.as_str(), *gate))
            .collect::<Vec<_>>()
    };

    let crate_root = lines_of(&crate_root_path);
    assert_eq!(
        line_gate(&crate_root, "mod wgpu_renderer;"),
        Some(WgpuGate::Wgpu),
        "{} must compile the WGPU renderer module only behind {WGPU_GATE}",
        crate_root_path.display()
    );
    assert_eq!(
        line_gate(
            &crate_root,
            "use wgpu_renderer::WindowsWgpuRenderer as WindowRenderer;"
        ),
        Some(WgpuGate::Wgpu),
        "{} must select the WGPU window renderer only behind {WGPU_GATE}",
        crate_root_path.display()
    );
    assert_eq!(
        line_gate(
            &crate_root,
            "use directx_renderer::DirectXRenderer as WindowRenderer;"
        ),
        Some(WgpuGate::NotWgpu),
        "{} must select the native DirectX window renderer behind {NOT_WGPU_GATE}",
        crate_root_path.display()
    );

    let window = lines_of(&window_path);
    assert_eq!(
        line_gate(&window, "renderer: RefCell<WindowRenderer>,"),
        Some(WgpuGate::Ungated),
        "{} must keep one RefCell<WindowRenderer> for both renderers",
        window_path.display()
    );
    assert_eq!(
        line_gate(&window, "DirectXRenderer::new("),
        Some(WgpuGate::NotWgpu),
        "{} must construct the native DirectX renderer behind {NOT_WGPU_GATE}",
        window_path.display()
    );
    assert_eq!(
        line_gate(&window, "WindowRenderer::new(hwnd, renderer_context)"),
        Some(WgpuGate::Wgpu),
        "{} must construct the WGPU renderer only behind {WGPU_GATE}",
        window_path.display()
    );

    let platform = lines_of(&platform_path);
    assert_eq!(
        line_gate(&platform, "DirectXDevices::new()"),
        Some(WgpuGate::NotWgpu),
        "{} must create DirectX devices for the default renderer behind {NOT_WGPU_GATE}",
        platform_path.display()
    );
}
