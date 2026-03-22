{
  description = "Standalone GPUI build";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      crane,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        inherit (pkgs) lib;

        # Get the latest stable toolchain via fenix
        toolchain = fenix.packages.${system}.stable.withComponents [
          "cargo"
          "rustc"
          "rust-src"
          "rustfmt"
          "clippy"
        ];

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        # Filter source to include .rs, shaders, and Cargo files
        src = lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            let
              base = baseNameOf path;
              ext = lib.last (lib.splitString "." base);
            in
            (lib.hasSuffix ".rs" base)
            || (lib.hasSuffix ".metal" base)
            || (lib.hasSuffix ".wgsl" base)
            || (lib.hasSuffix ".hlsl" base)
            || (builtins.elem base [
              "Cargo.toml"
              "Cargo.lock"
            ]);
        };

        # Shared dependencies for build and shell
        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = with pkgs; [
            cmake
            pkg-config
            rustPlatform.bindgenHook
          ];

          buildInputs =
            with pkgs;
            [
              fontconfig
              freetype
              openssl
              zlib
            ]
            ++ lib.optionals stdenv.isLinux [
              alsa-lib
              libdrm
              libgbm
              libxkbcommon
              vulkan-loader
              wayland
              xorg.libX11
              xorg.libxcb
            ]
            ++ lib.optionals stdenv.isDarwin [
              apple-sdk_15
              (darwinMinVersionHook "10.15")
            ];

          # Essential for GPUI to find Vulkan/Wayland libs at runtime on Linux
          env = lib.optionalAttrs pkgs.stdenv.isLinux {
            NIX_LDFLAGS = "-rpath ${
              lib.makeLibraryPath (
                with pkgs;
                [
                  vulkan-loader
                  wayland
                  libva
                ]
              )
            }";
          };

          # GPUI usually requires runtime shaders for practical use
          cargoExtraArgs = "--features gpui_platform/runtime_shaders";
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        gpui = craneLib.buildPackage (commonArgs // { inherit cargoArtifacts; });
      in
      {
        packages.default = gpui;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ gpui ];
          packages = [ toolchain ];

          # Realistic defaults for local development
          shellHook = ''
            export RUST_BACKTRACE=1
            export RUST_SRC_PATH="${toolchain}/lib/rustlib/src/rust/library"
          '';
        };
      }
    );
}
