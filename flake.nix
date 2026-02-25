{
  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      flake-utils,
      naersk,
      nixpkgs,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = (import nixpkgs) {
          inherit system;
          overlays = [
            (import rust-overlay)
          ];
        };

        naersk' = pkgs.callPackage naersk { };

        buildInputs = with pkgs; [
          libxkbcommon
          wayland
          vulkan-loader
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          xorg.libxcb
          xorg.libXrender
          xorg.libXfixes
          fontconfig
          freetype
          openssl
          libgit2
          alsa-lib
          zlib
          stdenv.cc.cc.lib
        ];

        nativeBuildInputs = with pkgs; [
          (pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "cargo"
              "rustc"
            ];
          })
          pkg-config
          cmake
          perl
          python3
        ];
      in
      rec {
        devShell = pkgs.mkShell {
          RUST_SRC_PATH = "${
            pkgs.rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" ];
            }
          }/lib/rustlib/src/rust/library";

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
          XDG_SESSION_TYPE = "wayland";
          shellHook = ''
            export WAYLAND_DISPLAY=''${WAYLAND_DISPLAY:-wayland-0}
            export XDG_RUNTIME_DIR=''${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
            export VK_LAYER_PATH="${pkgs.renderdoc}/lib:${pkgs.renderdoc}/lib64:${pkgs.renderdoc}/share/vulkan/implicit_layer.d:$VK_LAYER_PATH"
            export VK_INSTANCE_LAYERS="VK_LAYER_RENDERDOC_Capture:$VK_INSTANCE_LAYERS"
          '';

          nativeBuildInputs =
            with pkgs;
            [
              nixfmt
              cmake
              rustc
              rustfmt
              cargo
              clippy
              rust-analyzer
              vulkan-tools
              vulkan-loader
              vulkan-validation-layers
              renderdoc
            ]
            ++ buildInputs
            ++ nativeBuildInputs;
        };
      }
    );
}