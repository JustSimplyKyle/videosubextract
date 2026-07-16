{
  description = "libcosmic";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };
  outputs =
    {
      self,
      nixpkgs,
      utils,
      rust-overlay,
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
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
          src = ./.;
          cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ];
        };
        devShells.default = pkgs.mkShell rec {
          buildInputs =
            with pkgs;
            [
              rustToolchain
              cargo
              rustc
              rust-analyzer
              pkg-config
              openssl
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
            ]
            # opencv
            ++ [
              opencv
              stdenv.cc.cc
              clang
              libclang
            ]
            # icons
            ++ [
              cosmic-icons
              adwaita-icon-theme
              hicolor-icon-theme
            ];

          XDG_DATA_DIRS = pkgs.lib.concatStringsSep ":" [
            "${pkgs.cosmic-icons}/share"
            # "${pkgs.adwaita-icon-theme}/share"
            # "${pkgs.hicolor-icon-theme}/share"
            "$XDG_DATA_DIRS" # preserve any existing paths
          ];

          COSMIC_ICONS = "${pkgs.cosmic-icons}/share";

          RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
          FONT_PATH = "${pkgs.noto-fonts-cjk-sans}/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc";
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          LD_LIBRARY_PATH = "$LD_LIBRARY_PATH:${pkgs.lib.makeLibraryPath buildInputs}";
        };
      }
    );
}
