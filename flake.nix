{
  description = "mbr markdown browser";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-25.11-darwin";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      overlays = [(import rust-overlay)];
      pkgs = import nixpkgs {
        inherit system overlays;
        config.allowUnfree = true;
      };
      rusttoolchain =
        pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      # ffmpeglibs = pkgs.ffmpeg-headless.override {
      #   buildAvcodec = true;
      #   buildAvdevice = true;
      #   buildAvfilter = true;
      #   buildAvformat = true;
      #   buildAvutil = true;
      # };
    in rec {
      # Build frontend components first
      packages.mbr-components = pkgs.buildNpmPackage {
        pname = "mbr-components";
        version = "0.1";
        src = ./components;
        npmDepsHash = "sha256-kf1UObyt2f9WJLVHWFcEC8NMM4Mg7cW46Dqcv1EQns8=";
        buildPhase = ''
          npm run build
        '';
        installPhase = ''
          mkdir -p $out
          cp -r dist/* $out/
        '';
      };

      # `nix build`
      packages.mbr-cli = pkgs.rustPlatform.buildRustPackage rec {
        pname = "mbr";
        version = "0.1";
        cargoLock.lockFile = ./Cargo.lock;
        src = pkgs.lib.cleanSource ./.;
        preBuild = ''
          mkdir -p components/dist
          cp -r ${packages.mbr-components}/* components/dist/
        '';

        nativeBuildInputs = with pkgs;
          [
            pkg-config
            llvmPackages.libclang
          ]
          ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin (with pkgs; [
            apple-sdk
          ]));

        buildInputs = with pkgs; [
          ffmpeg_7-full.dev
        ];

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        FFMPEG_DIR = "${pkgs.ffmpeg_7-full.dev}";
      };
      defaultPackage = packages.mbr-cli;

      # `nix run`
      apps.mbr-cli = flake-utils.lib.mkApp {drv = packages.mbr-cli;};
      defaultApp = apps.mbr-cli;

      # nix develop
      devShell = pkgs.mkShell {
        buildInputs =
          [
            rusttoolchain
            pkgs.nodejs_24
            pkgs.ffmpeg_7-full.dev
            pkgs.pkg-config
            pkgs.cargo-watch
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.apple-sdk
            #pkgs.darwin.xcode_26
          ];
        PKG_CONFIG_PATH = "${pkgs.ffmpeg-headless.dev}/lib/pkgconfig";
        LD_LIBRARY_PATH = "${pkgs.stdenv.cc.cc.lib}/lib";
      };
    });
}
