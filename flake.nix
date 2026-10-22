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

        toolchain = fenix.packages.${system}.combine [
          (fenix.packages.${system}.latest.withComponents [
            "cargo"
            "rustc"
            "rust-src"
            "rustfmt"
            "clippy"
          ])
          fenix.packages.${system}.targets.wasm32-unknown-unknown.latest.rust-std
        ];

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        src = lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter =
            path: type:
            (craneLib.filterCargoSources path type)
            || (lib.hasSuffix ".metal" path)
            || (lib.hasSuffix ".wgsl" path)
            || (lib.hasSuffix ".hlsl" path)
            || (lib.hasSuffix ".glsl" path);
        };

        linuxLibs = with pkgs; [
          alsa-lib
          libdrm
          mesa # provides libgbm
          libxkbcommon
          libva
          vulkan-loader
          wayland
          xorg.libX11
          xorg.libxcb
        ];

        commonArgs = {
          pname = "gpui-ce";
          version = "0.3.3";

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
            ++ lib.optionals stdenv.isLinux linuxLibs
            ++ lib.optionals stdenv.isDarwin [
              apple-sdk_15
              (darwinMinVersionHook "11.0")
            ];

          env = lib.optionalAttrs pkgs.stdenv.isLinux {
            LD_LIBRARY_PATH = lib.makeLibraryPath linuxLibs;
          };

          cargoExtraArgs = "--features runtime_shaders";
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        gpui = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );
      in
      {
        packages.default = gpui;

        devShells.default = pkgs.mkShell {
          # Rustup proxies support `cargo +toolchain` for MSRV and WASM-atomics CI checks.
          # Include the native build dependencies in the same shell.
          packages = [
            pkgs.rustup
            pkgs.cargo-machete
            pkgs.taplo
            pkgs.typos
            pkgs.just
            pkgs.nushell
            pkgs.cmake
            pkgs.pkg-config
            pkgs.rustPlatform.bindgenHook
            pkgs.fontconfig
            pkgs.freetype
            pkgs.openssl
            pkgs.zlib
          ] ++ lib.optionals pkgs.stdenv.isDarwin [
            pkgs.apple-sdk_15
            (pkgs.darwinMinVersionHook "11.0")
          ] ++ lib.optionals pkgs.stdenv.isLinux linuxLibs;

          shellHook = ''
            export RUST_BACKTRACE=1
            ${lib.optionalString pkgs.stdenv.isDarwin ''
              # Use the real Xcode SDK rather than Nix's apple-sdk stub so xcrun
              # finds the system Metal compiler for gpui_macos's .metal shaders.
              export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
              # Prefer /usr/bin/xcrun over Nix's stub, which cannot find the Metal toolchain.
              mkdir -p /tmp/gpui-ce-bin
              ln -sf /usr/bin/xcrun /tmp/gpui-ce-bin/xcrun
              export PATH="/tmp/gpui-ce-bin:$PATH"
            ''}
            ${lib.optionalString pkgs.stdenv.isLinux ''
              export LD_LIBRARY_PATH="${lib.makeLibraryPath linuxLibs}:$LD_LIBRARY_PATH"
            ''}
          '';
        };
      }
    );
}
