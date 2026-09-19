{
  description = "Extract hard-coded video subtitles with a Dioxus desktop UI";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    dioxus.url = "github:DioxusLabs/dioxus/main";
    dioxus.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs =
    {
      self,
      nixpkgs,
      utils,
      rust-overlay,
      dioxus,
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "rustc-codegen-cranelift-preview"
          ];
        };
        # The Nix clang wrapper searches GCC's static-only output before its
        # shared-library output.  That makes `-lstdc++` select libstdc++.a;
        # mold can then emit an incomplete C++ vtable when shared C++
        # dependencies are present.  Put the shared runtime first while still
        # using mold for the actual link.
        clangMold = pkgs.writeShellScriptBin "clang-mold" ''
          exec ${pkgs.clang}/bin/clang \
            -L${pkgs.stdenv.cc.cc.lib}/lib \
            -fuse-ld=mold \
            "$@"
        '';
        # Cranelift writes inline assembly to sibling `*.rcgu.asm.o` files.
        # Dioxus ThinLink otherwise ignores them and produces patches with
        # unresolved symbols from crates such as rustix and event-listener.
        dioxusCli = dioxus.packages.${system}.dioxus-cli.overrideAttrs (old: {
          patches = (old.patches or [ ]) ++ [
            ./patches/dioxus-cli-cranelift-asm-objects.patch
          ];
        });
        videosubfinderHelixLanguages = pkgs.writeText "videosubfinder-languages.toml" ''
          [language-server.clangd]
          args = ["--query-driver=${pkgs.stdenv.cc}/bin/g++"]
        '';
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
          src = ./.;
          cargoHash = "sha256-Ek8V4obqKj2pYbPd3gQTq/ucx8PKTGGNnCTCiAgPLAM=";
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = with pkgs; [
            openssl
            gtk3
            webkitgtk_4_1
            libsoup_3
          ];
        };
        devShells.default = pkgs.mkShell rec {
          buildInputs =
            with pkgs;
            [
              rustToolchain
              cargo
              rustc
              rust-analyzer
              cmake
              pkg-config
              openssl
              gtk3
              webkitgtk_4_1
              libsoup_3
              xdotool
              libxkbcommon
              wayland
              libGL
              libxkbcommon
              glib
              dbus
              just
              ffmpeg.dev
              opencc
              mold
              clangMold
              sccache
              samply
              vulkan-loader
              tbb
              dioxusCli
              linuxdeploy
            ]
            # opencv
            ++ [
              opencv
              stdenv.cc.cc
              clang
              libclang
            ]
            ;

          # RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
          FONT_PATH = "${pkgs.noto-fonts-cjk-sans}/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc";
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          LD_LIBRARY_PATH = "$LD_LIBRARY_PATH:${pkgs.lib.makeLibraryPath buildInputs}";

          shellHook = ''
            mkdir -p third_party/videosubfinder-src/.helix
            ln -sfn ${videosubfinderHelixLanguages} third_party/videosubfinder-src/.helix/languages.toml
          '';
        };
      }
    );
}
