{
  description = "mbr markdown browser";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-25.05-darwin";
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
      pkgs = import nixpkgs {inherit system overlays;};
      rusttoolchain =
        pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
    in rec {
      # `nix build`
      packages.mbr-cli = pkgs.rustPlatform.buildRustPackage {
        pname = "mbr";
        version = "0.1";
        cargoLock.lockFile = ./Cargo.lock;
        src = pkgs.lib.cleanSource ./.;
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
            pkgs.nodejs-slim_24
            pkgs.pkg-config
            pkgs.ffmpeg-headless
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin (with pkgs; [
            # libiconv
            darwin.apple_sdk.frameworks.Security
          ]);
        LD_LIBRARY_PATH = "${pkgs.stdenv.cc.cc.lib}/lib";
      };
    });
}
