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

      version = "0.1.0";

      # Info.plist content for macOS app bundle
      infoPlist = pkgs.writeText "Info.plist" ''
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
          <key>CFBcargo watch -q -c -x 'run --release -- -s README.md'undleDevelopmentRegion</key>
          <string>en</string>
          <key>CFBundleDisplayName</key>
          <string>MBR</string>
          <key>CFBundleExecutable</key>
          <string>mbr</string>
          <key>CFBundleIconFile</key>
          <string>AppIcon</string>
          <key>CFBundleIdentifier</key>
          <string>com.zmre.mbr</string>
          <key>CFBundleInfoDictionaryVersion</key>
          <string>6.0</string>
          <key>CFBundleName</key>
          <string>MBR</string>
          <key>CFBundlePackageType</key>
          <string>APPL</string>
          <key>CFBundleShortVersionString</key>
          <string>${version}</string>
          <key>CFBundleVersion</key>
          <string>${version}</string>
          <key>CFBundleSignature</key>
          <string>????</string>
          <key>CFBundleSupportedPlatforms</key>
          <array>
            <string>MacOSX</string>
          </array>
          <key>CFBundleDocumentTypes</key>
          <array>
            <dict>
              <key>CFBundleTypeExtensions</key>
              <array>
                <string>markdown</string>
                <string>md</string>
                <string>mdoc</string>
                <string>mdown</string>
                <string>mdtext</string>
                <string>mdtxt</string>
                <string>mdwn</string>
                <string>mkd</string>
                <string>mkdn</string>
              </array>
              <key>CFBundleTypeName</key>
              <string>Markdown document</string>
              <key>CFBundleTypeRole</key>
              <string>Viewer</string>
            </dict>
          </array>
          <key>CFBundleURLTypes</key>
          <array>
            <dict>
              <key>CFBundleTypeRole</key>
              <string>Viewer</string>
              <key>CFBundleURLName</key>
              <string>MBR</string>
              <key>CFBundleURLSchemes</key>
              <array>
                <string>mbr</string>
              </array>
            </dict>
          </array>
          <key>LSApplicationCategoryType</key>
          <string>public.app-category.productivity</string>
          <key>LSMinimumSystemVersion</key>
          <string>10.13</string>
          <key>NSHighResolutionCapable</key>
          <true/>
          <key>NSHumanReadableCopyright</key>
          <string>Copyright © 2025 Patrick Walsh. All rights reserved.</string>
        </dict>
        </plist>
      '';

      # Platform-specific arch string for release artifacts
      archString =
        if system == "aarch64-darwin"
        then "macos-arm64"
        else if system == "x86_64-darwin"
        then "macos-x86_64"
        else if system == "aarch64-linux"
        then "linux-arm64"
        else if system == "x86_64-linux"
        then "linux-x86_64"
        else system;
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

      # Main package: CLI binary + macOS app bundle (signed on darwin)
      packages.mbr = pkgs.rustPlatform.buildRustPackage {
        pname = "mbr";
        inherit version;
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
          ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.apple-sdk
          ]);

        buildInputs = with pkgs; [
          ffmpeg_7-full.dev
        ];

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        FFMPEG_DIR = "${pkgs.ffmpeg_7-full.dev}";

        # Create CLI binary (all platforms) + .app bundle (macOS only, signed)
        postInstall = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
          # Create macOS .app bundle structure
          mkdir -p $out/Applications/MBR.app/Contents/{MacOS,Resources}

          # Copy binary to app bundle
          cp $out/bin/mbr $out/Applications/MBR.app/Contents/MacOS/mbr

          # Install Info.plist
          cp ${infoPlist} $out/Applications/MBR.app/Contents/Info.plist

          # Copy icon
          cp ${./macos/AppIcon.icns} $out/Applications/MBR.app/Contents/Resources/AppIcon.icns

          # Ad-hoc sign the entire bundle (signs binary and seals resources)
          /usr/bin/codesign --force --sign - --deep $out/Applications/MBR.app
        '';

        meta = with pkgs.lib; {
          description = "A markdown viewer, browser, and static site generator";
          homepage = "https://github.com/zmre/mbr";
          license = licenses.mit;
          maintainers = [];
          mainProgram = "mbr";
          platforms = platforms.unix;
        };
      };

      packages.default = packages.mbr;

      # Release package: creates distributable archives from the built package
      packages.release =
        pkgs.runCommand "mbr-release-${version}" {
          nativeBuildInputs = [pkgs.gnutar pkgs.gzip];
        } (
          if pkgs.stdenv.isDarwin
          then ''
            mkdir -p $out

            # Create .app bundle archive
            tar -czvf $out/mbr-${archString}.tar.gz \
              -C ${packages.mbr}/Applications \
              MBR.app

            # Create CLI-only archive
            tar -czvf $out/mbr-cli-${archString}.tar.gz \
              -C ${packages.mbr}/bin \
              mbr

            # Create checksums
            cd $out
            sha256sum *.tar.gz > SHA256SUMS

            echo ""
            echo "Release artifacts:"
            ls -lh $out/
          ''
          else ''
            mkdir -p $out

            # Create CLI archive (Linux)
            tar -czvf $out/mbr-${archString}.tar.gz \
              -C ${packages.mbr}/bin \
              mbr

            # Create checksums
            cd $out
            sha256sum *.tar.gz > SHA256SUMS

            echo ""
            echo "Release artifacts:"
            ls -lh $out/
          ''
        );

      # Apps
      apps.default = flake-utils.lib.mkApp {drv = packages.mbr;};
      apps.mbr = apps.default;

      # Release app: builds release and shows output location
      apps.release = {
        type = "app";
        program = "${pkgs.writeShellApplication {
          name = "mbr-release";
          text = ''
            echo "Building release artifacts..."
            echo ""
            echo "Release output: ${packages.release}"
            echo ""
            echo "Contents:"
            ls -lh ${packages.release}/
            echo ""
            echo "To copy to local directory:"
            echo "  cp -r ${packages.release}/* ./dist/"
          '';
        }}/bin/mbr-release";
      };

      # Development shell
      devShells.default = pkgs.mkShell {
        buildInputs =
          [
            rusttoolchain
            pkgs.nodejs_24
            pkgs.bun
            pkgs.ffmpeg_7-full.dev
            pkgs.pkg-config
            pkgs.cargo-watch
            pkgs.llvmPackages.libclang
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.apple-sdk
          ];
        PKG_CONFIG_PATH = "${pkgs.ffmpeg-headless.dev}/lib/pkgconfig";
        LD_LIBRARY_PATH = "${pkgs.stdenv.cc.cc.lib}/lib";
        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        FFMPEG_DIR = "${pkgs.ffmpeg_7-full.dev}";
      };
    });
}
