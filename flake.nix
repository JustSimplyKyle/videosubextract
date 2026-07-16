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
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };
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
              adwaita-icon-theme
              hicolor-icon-theme
            ];

          XDG_DATA_DIRS = pkgs.lib.concatStringsSep ":" [
            "${pkgs.adwaita-icon-theme}/share"
            "${pkgs.hicolor-icon-theme}/share"
            "$XDG_DATA_DIRS" # preserve any existing paths
          ];

          # OPENCC_DATA_PATH = "${pkgs.opencc}/share/opencc";

          FONT_PATH = "${pkgs.noto-fonts-cjk-sans}/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc";
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          LD_LIBRARY_PATH = "$LD_LIBRARY_PATH:${pkgs.lib.makeLibraryPath buildInputs}";
        };
      }
    );
}
