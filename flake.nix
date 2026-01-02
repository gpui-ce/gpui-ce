{
  description = "GPUI-CE - Community fork of Zed's GPU-accelerated UI framework";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        # Native build dependencies
        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
          cmake
          ninja
          python3
          nasm
        ];

        # Build dependencies (libraries)
        buildInputs = with pkgs; [
          # X11 dependencies
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi
          xorg.libXext
          xorg.libXfixes
          xorg.libXrender
          xorg.libXtst
          xorg.libxcb
          xorg.xcbutilwm
          xorg.xcbutilkeysyms
          xorg.xcbutilimage

          # Wayland dependencies
          wayland
          wayland-protocols
          wayland-scanner
          libxkbcommon

          # Font/text rendering
          fontconfig
          freetype

          # Image libraries
          libpng
          libjpeg

          # System libraries
          libgit2
          openssl
          zlib
          zstd

          # Vulkan/GPU libraries
          vulkan-loader
          vulkan-headers
          vulkan-tools

          # Audio/media
          alsa-lib
          pipewire
          dbus
        ];

      in
      {
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;

          shellHook = ''
            export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath buildInputs}:$LD_LIBRARY_PATH
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "gpui-ce";
          version = "0.3.3";

          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          inherit nativeBuildInputs buildInputs;

          cargoBuildFlags = [ "--features" "wayland" ];
          cargoTestFlags = [ "--features" "wayland" ];

          # The package is a library, so we don't need to install binaries
          doCheck = false;
        };
      }
    );
}
