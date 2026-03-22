#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

use std::env;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(gles)");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    match target_os.as_str() {
        "macos" => {
            #[cfg(target_os = "macos")]
            macos::build();
        }
        "windows" => {
            #[cfg(target_os = "windows")]
            windows::build();
        }
        _ => {}
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        env,
        path::{Path, PathBuf},
    };

    use cbindgen::Config;

    pub fn build() {
        let header_path = generate_shader_bindings();

        #[cfg(feature = "runtime_shaders")]
        emit_stitched_shaders(&header_path);
        #[cfg(not(feature = "runtime_shaders"))]
        compile_metal_shaders(&header_path);
    }

    fn generate_shader_bindings() -> PathBuf {
        let output_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("scene.h");
        let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

        let mut config = Config {
            include_guard: Some("SCENE_H".into()),
            language: cbindgen::Language::C,
            no_includes: true,
            ..Default::default()
        };
        config.export.include.extend([
            "Bounds".into(),
            "Corners".into(),
            "Edges".into(),
            "Size".into(),
            "Pixels".into(),
            "PointF".into(),
            "Hsla".into(),
            "ContentMask".into(),
            "Uniforms".into(),
            "AtlasTile".into(),
            "PathRasterizationInputIndex".into(),
            "PathVertex_ScaledPixels".into(),
            "PathRasterizationVertex".into(),
            "ShadowInputIndex".into(),
            "Shadow".into(),
            "QuadInputIndex".into(),
            "Underline".into(),
            "UnderlineInputIndex".into(),
            "Quad".into(),
            "BorderStyle".into(),
            "SpriteInputIndex".into(),
            "MonochromeSprite".into(),
            "PolychromeSprite".into(),
            "PathSprite".into(),
            "SurfaceInputIndex".into(),
            "SurfaceBounds".into(),
            "TransformationMatrix".into(),
        ]);
        config.no_includes = true;
        config.enumeration.prefix_with_name = true;

        let mut builder = cbindgen::Builder::new();

        // Source files that define types used in shaders
        let src_paths = [
            crate_dir.join("src/scene.rs"),
            crate_dir.join("src/geometry.rs"),
            crate_dir.join("src/color.rs"),
            crate_dir.join("src/window.rs"),
            crate_dir.join("src/platform.rs"),
            crate_dir.join("src/platform/mac/metal_renderer.rs"),
        ];

        for src_path in &src_paths {
            println!("cargo:rerun-if-changed={}", src_path.display());
            builder = builder.with_src(src_path);
        }

        builder
            .with_config(config)
            .generate()
            .expect("Unable to generate bindings")
            .write_to_file(&output_path);

        output_path
    }

    /// To enable runtime compilation, we need to "stitch" the shaders file with the generated header
    /// so that it is self-contained.
    #[cfg(feature = "runtime_shaders")]
    fn emit_stitched_shaders(header_path: &Path) {
        fn stitch_header(header: &Path, shader_path: &Path) -> std::io::Result<PathBuf> {
            let header_contents = std::fs::read_to_string(header)?;
            let shader_contents = std::fs::read_to_string(shader_path)?;
            let stitched_contents = format!("{header_contents}\n{shader_contents}");
            let out_path =
                PathBuf::from(env::var("OUT_DIR").unwrap()).join("stitched_shaders.metal");
            std::fs::write(&out_path, stitched_contents)?;
            Ok(out_path)
        }
        let shader_source_path = "./src/platform/mac/shaders.metal";
        let shader_path = PathBuf::from(shader_source_path);
        stitch_header(header_path, &shader_path).unwrap();
        println!("cargo:rerun-if-changed={}", &shader_source_path);
    }

    #[cfg(not(feature = "runtime_shaders"))]
    fn compile_metal_shaders(header_path: &Path) {
        use std::process::{self, Command};
        let shader_path = "./src/platform/mac/shaders.metal";
        let air_output_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("shaders.air");
        let metallib_output_path =
            PathBuf::from(env::var("OUT_DIR").unwrap()).join("shaders.metallib");
        println!("cargo:rerun-if-changed={}", shader_path);

        let output = Command::new("xcrun")
            .args([
                "-sdk",
                "macosx",
                "metal",
                "-gline-tables-only",
                "-mmacosx-version-min=10.15.7",
                "-MO",
                "-c",
                shader_path,
                "-include",
                (header_path.to_str().unwrap()),
                "-o",
            ])
            .arg(&air_output_path)
            .output()
            .unwrap();

        if !output.status.success() {
            println!(
                "cargo::error=metal shader compilation failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            process::exit(1);
        }

        let output = Command::new("xcrun")
            .args(["-sdk", "macosx", "metallib"])
            .arg(air_output_path)
            .arg("-o")
            .arg(metallib_output_path)
            .output()
            .unwrap();

        if !output.status.success() {
            println!(
                "cargo::error=metallib compilation failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            process::exit(1);
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    pub fn build() {
        #[cfg(not(debug_assertions))]
        shader_compilation::compile_shaders();

        #[cfg(feature = "windows-manifest")]
        embed_resource();
    }

    #[cfg(feature = "windows-manifest")]
    fn embed_resource() {
        let manifest = std::path::Path::new("resources/windows/gpui.manifest.xml");
        let rc_file = std::path::Path::new("resources/windows/gpui.rc");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rerun-if-changed={}", rc_file.display());
        embed_resource::compile(rc_file, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }

    #[cfg(not(debug_assertions))]
    mod shader_compilation {
        use std::{
            fs,
            io::Write,
            path::{Path, PathBuf},
            process::{self, Command},
        };

        pub fn compile_shaders() {
            let shader_path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("src/platform/windows/shaders.hlsl");
            let out_dir = std::env::var("OUT_DIR").unwrap();

            println!("cargo:rerun-if-changed={}", shader_path.display());

            let fxc_path = find_fxc_compiler();

            let modules = [
                "quad",
                "shadow",
                "path_rasterization",
                "path_sprite",
                "underline",
                "monochrome_sprite",
                "subpixel_sprite",
                "polychrome_sprite",
            ];

            let rust_binding_path = format!("{}/shaders_bytes.rs", out_dir);
            if Path::new(&rust_binding_path).exists() {
                fs::remove_file(&rust_binding_path)
                    .expect("Failed to remove existing Rust binding file");
            }
            for module in modules {
                compile_shader_for_module(
                    module,
                    &out_dir,
                    &fxc_path,
                    shader_path.to_str().unwrap(),
                    &rust_binding_path,
                );
            }

            {
                let shader_path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                    .join("src/platform/windows/color_text_raster.hlsl");
                compile_shader_for_module(
                    "emoji_rasterization",
                    &out_dir,
                    &fxc_path,
                    shader_path.to_str().unwrap(),
                    &rust_binding_path,
                );
            }
        }

        fn find_latest_windows_sdk_binary(
            binary: &str,
        ) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
            let key = windows_registry::LOCAL_MACHINE
                .open("SOFTWARE\\WOW6432Node\\Microsoft\\Microsoft SDKs\\Windows\\v10.0")?;

            let install_folder: String = key.get_string("InstallationFolder")?;
            let install_folder_bin = Path::new(&install_folder).join("bin");

            let mut versions: Vec<_> = std::fs::read_dir(&install_folder_bin)?
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect();

            versions.sort_by_key(|s| {
                s.split('.')
                    .filter_map(|p| p.parse().ok())
                    .collect::<Vec<u32>>()
            });

            let arch = match std::env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                _ => Err(format!(
                    "Unsupported architecture: {}",
                    std::env::consts::ARCH
                ))?,
            };

            if let Some(highest_version) = versions.last() {
                return Ok(Some(
                    install_folder_bin
                        .join(highest_version)
                        .join(arch)
                        .join(binary),
                ));
            }

            Ok(None)
        }

        fn find_fxc_compiler() -> String {
            if let Ok(path) = std::env::var("GPUI_FXC_PATH")
                && Path::new(&path).exists()
            {
                return path;
            }

            if let Ok(output) = std::process::Command::new("where.exe")
                .arg("fxc.exe")
                .output()
                && output.status.success()
            {
                let path = String::from_utf8_lossy(&output.stdout);
                return path.trim().to_string();
            }

            if let Ok(Some(path)) = find_latest_windows_sdk_binary("fxc.exe") {
                return path.to_string_lossy().into_owned();
            }

            panic!("Failed to find fxc.exe");
        }

        fn compile_shader_for_module(
            module: &str,
            out_dir: &str,
            fxc_path: &str,
            shader_path: &str,
            rust_binding_path: &str,
        ) {
            let output_file = format!("{}/{}_vs.h", out_dir, module);
            let const_name = format!("{}_VERTEX_BYTES", module.to_uppercase());
            compile_shader_impl(
                fxc_path,
                &format!("{module}_vertex"),
                &output_file,
                &const_name,
                shader_path,
                "vs_4_1",
            );
            generate_rust_binding(&const_name, &output_file, rust_binding_path);

            let output_file = format!("{}/{}_ps.h", out_dir, module);
            let const_name = format!("{}_FRAGMENT_BYTES", module.to_uppercase());
            compile_shader_impl(
                fxc_path,
                &format!("{module}_fragment"),
                &output_file,
                &const_name,
                shader_path,
                "ps_4_1",
            );
            generate_rust_binding(&const_name, &output_file, rust_binding_path);
        }

        fn compile_shader_impl(
            fxc_path: &str,
            entry_point: &str,
            output_path: &str,
            var_name: &str,
            shader_path: &str,
            target: &str,
        ) {
            let output = Command::new(fxc_path)
                .args([
                    "/T",
                    target,
                    "/E",
                    entry_point,
                    "/Fh",
                    output_path,
                    "/Vn",
                    var_name,
                    "/O3",
                    shader_path,
                ])
                .output();

            match output {
                Ok(result) => {
                    if result.status.success() {
                        return;
                    }
                    println!(
                        "cargo::error=Shader compilation failed for {}:\n{}",
                        entry_point,
                        String::from_utf8_lossy(&result.stderr)
                    );
                    process::exit(1);
                }
                Err(e) => {
                    println!("cargo::error=Failed to run fxc for {}: {}", entry_point, e);
                    process::exit(1);
                }
            }
        }

        fn generate_rust_binding(const_name: &str, head_file: &str, output_path: &str) {
            let header_content = fs::read_to_string(head_file).expect("Failed to read header file");
            let const_definition = {
                let global_var_start = header_content.find("const BYTE").unwrap();
                let global_var = &header_content[global_var_start..];
                let equal = global_var.find('=').unwrap();
                global_var[equal + 1..].trim()
            };
            let rust_binding = format!(
                "const {}: &[u8] = &{}\n",
                const_name,
                const_definition.replace('{', "[").replace('}', "]")
            );
            let mut options = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(output_path)
                .expect("Failed to open Rust binding file");
            options
                .write_all(rust_binding.as_bytes())
                .expect("Failed to write Rust binding file");
        }
    }
}
