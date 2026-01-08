{
  nixConfig = {
    extra-substituters = [
      "https://cache.nixos.org"
      "https://nix-community.cachix.org"
      "https://zmre.cachix.org"
    ];
    extra-trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      "zmre.cachix.org-1:WIE1U2a16UyaUVr+Wind0JM6pEXBe43PQezdPKoDWLE="
    ];
  };
  description = "mbr markdown browser";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-25.11-darwin";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    crane,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      overlays = [(import rust-overlay)];
      pkgs = import nixpkgs {
        inherit system overlays;
        config.allowUnfree = true;
      };

      # Get rust toolchain from rust-toolchain.toml
      rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      # Create crane lib with our toolchain
      craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

      # Read version from Cargo.toml - single source of truth
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      version = cargoToml.package.version;

      # Info.plist content for macOS app bundle
      infoPlist = pkgs.writeText "Info.plist" ''
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
          <key>CFBundleDevelopmentRegion</key>
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

      # Source filtering - include Rust sources, templates, and embedded assets
      src = pkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: type:
          (craneLib.filterCargoSources path type)
          || (builtins.match ".*templates.*" path != null)
          || (builtins.match ".*\\.md$" path != null)
          || (builtins.match ".*\\.png$" path != null)
          || (builtins.match ".*\\.icns$" path != null)
          || (builtins.match ".*\\.udl$" path != null) # UniFFI interface definitions
          # QuickLook extension sources
          || (builtins.match ".*\\.swift$" path != null)
          || (builtins.match ".*\\.plist$" path != null)
          || (builtins.match ".*\\.entitlements$" path != null)
          || (builtins.match ".*\\.modulemap$" path != null)
          || (builtins.match ".*\\.h$" path != null) # C headers for FFI
          || (builtins.match ".*/quicklook/project\\.yml$" path != null)
          || (builtins.match ".*/quicklook/build\\.sh$" path != null);
      };

      # Shared native build inputs
      commonNativeBuildInputs = with pkgs;
        [
          pkg-config
          llvmPackages.libclang
          typescript
          nodejs_24
          bun
        ]
        ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.apple-sdk
        ]);

      # Shared build inputs
      commonBuildInputs = with pkgs;
        [
          ffmpeg_7-full.dev
        ]
        ++ (pkgs.lib.optionals pkgs.stdenv.isLinux [
          # Required by wry/tao for Linux webview
          gtk3
          glib
          webkitgtk_4_1
          libsoup_3
          cairo
          pango
          gdk-pixbuf
          atk
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          xdotool # provides libxdo needed by wry/tao
        ]);

      # Shared environment variables for builds
      commonEnvVars = {
        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        FFMPEG_DIR = "${pkgs.ffmpeg_7-full.dev}";
        # Tell bindgen where to find glibc headers on Linux (required by ffmpeg-sys-next)
        BINDGEN_EXTRA_CLANG_ARGS =
          pkgs.lib.optionalString pkgs.stdenv.isLinux
          "-isystem ${pkgs.stdenv.cc.libc.dev}/include";
      };

      # Common arguments shared between builds
      commonArgs =
        commonEnvVars
        // {
          inherit src;
          strictDeps = true;
          pname = "mbr";
          inherit version;
          nativeBuildInputs = commonNativeBuildInputs;
          buildInputs = commonBuildInputs;
        };

      # Build dependencies only (cached separately from source changes)
      cargoArtifacts = craneLib.buildDepsOnly (commonArgs
        // {
          # Dummy source for dependency-only build
          src = craneLib.cleanCargoSource ./.;
          preBuild = ''
            # Create empty component files for dependency resolution
            # Must match the actual file names produced by vite build (see vite.config.ts)
            mkdir -p templates/components-js
            touch templates/components-js/mbr-components.js
          '';
        });
    in rec {
      # Build frontend components first
      packages.mbr-components = pkgs.buildNpmPackage {
        pname = "mbr-components";
        inherit version;
        src = ./components;
        # npmDepsHash = pkgs.lib.fakeHash;
        npmDepsHash = "sha256-qWsHQidGobEZmqTlYd+wHyEQnshV6fF3z/Sr5fTaNS0=";
        buildPhase = ''
          npm run build
        '';
        installPhase = ''
          mkdir -p $out
          cp -r ../templates/components-js/* $out/
        '';
      };

      # QuickLook staticlib: builds libmbr.a without GUI/ffmpeg for sandbox compatibility
      packages.mbr-quicklook-staticlib = pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin (
        craneLib.buildPackage (commonArgs
          // {
            inherit cargoArtifacts;
            pname = "mbr-quicklook-staticlib";
            # Build only the staticlib without GUI or media-metadata features
            # These would pull in SDL/ffmpeg which crash in QuickLook sandbox
            # Enable ffi feature for UniFFI bindings (required for Swift interop)
            cargoExtraArgs = "--locked --no-default-features --features ffi --lib";

            preBuild = ''
              mkdir -p templates/components-js
              cp -r ${packages.mbr-components}/* templates/components-js/
            '';

            # Only install the static library
            installPhaseCommand = ''
              mkdir -p $out/lib
              cp target/release/libmbr.a $out/lib/
            '';
          })
      );

      # QuickLook extension: builds the .appex using swiftc directly
      packages.mbr-quicklook = pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin (
        pkgs.stdenv.mkDerivation {
          pname = "mbr-quicklook";
          inherit version;
          inherit src;

          nativeBuildInputs = [
            pkgs.swift
            pkgs.apple-sdk
          ];

          buildPhase = ''
            mkdir -p build/MBRPreview.appex/Contents/MacOS

            # Compile the QuickLook extension using swiftc from nixpkgs
            # App extensions should be MH_EXECUTE (executables), not MH_BUNDLE
            # -parse-as-library: Don't look for main() function
            # -application-extension: Mark as app extension (required for sandboxing)
            # -e _NSExtensionMain: Use extension entry point instead of _main
            swiftc \
              -O \
              -parse-as-library \
              -application-extension \
              -target arm64-apple-macos14.0 \
              -sdk ${pkgs.apple-sdk}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk \
              -L ${packages.mbr-quicklook-staticlib}/lib \
              -lmbr \
              -framework Foundation \
              -framework CoreFoundation \
              -framework Security \
              -framework SystemConfiguration \
              -framework Cocoa \
              -framework QuickLookUI \
              -framework Quartz \
              -framework WebKit \
              -framework ExtensionKit \
              -module-name MBRPreview \
              -Xlinker -e -Xlinker _NSExtensionMain \
              -o build/MBRPreview.appex/Contents/MacOS/MBRPreview \
              -I quicklook/Generated \
              -Xcc -fmodule-map-file=quicklook/Generated/mbrFFI.modulemap \
              quicklook/Generated/mbr.swift \
              quicklook/MBRPreview/PreviewViewController.swift

            # Copy Info.plist to complete the .appex bundle structure
            cp quicklook/MBRPreview/Info.plist build/MBRPreview.appex/Contents/Info.plist
          '';

          installPhase = ''
            mkdir -p $out
            cp -R build/MBRPreview.appex $out/
          '';
        }
      );

      # Main package: CLI binary + macOS app bundle (signed on darwin)
      packages.mbr = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          cargoExtraArgs = "--locked";

          preBuild = ''
            mkdir -p templates/components-js
            cp -r ${packages.mbr-components}/* templates/components-js/
          '';

          # Create CLI binary (all platforms) + .app bundle (macOS only, signed)
          postInstall = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
            # Create macOS .app bundle structure
            mkdir -p $out/Applications/MBR.app/Contents/{MacOS,Resources,PlugIns}

            # Copy binary to app bundle
            cp $out/bin/mbr $out/Applications/MBR.app/Contents/MacOS/mbr

            # Install Info.plist
            cp ${infoPlist} $out/Applications/MBR.app/Contents/Info.plist

            # Copy icon
            cp ${./macos/AppIcon.icns} $out/Applications/MBR.app/Contents/Resources/AppIcon.icns

            # Copy QuickLook extension (make writable for codesigning)
            cp -R ${packages.mbr-quicklook}/MBRPreview.appex $out/Applications/MBR.app/Contents/PlugIns/
            chmod -R u+w $out/Applications/MBR.app/Contents/PlugIns/MBRPreview.appex

            # Sign the extension first with its entitlements, then sign the app bundle
            /usr/bin/codesign --force --sign - \
              --entitlements ${./quicklook/MBRPreview/MBRPreview.entitlements} \
              $out/Applications/MBR.app/Contents/PlugIns/MBRPreview.appex
            /usr/bin/codesign --force --sign - $out/Applications/MBR.app
          '';

          meta = with pkgs.lib; {
            description = "A markdown viewer, browser, and static site generator";
            homepage = "https://github.com/zmre/mbr";
            license = licenses.mit;
            maintainers = [];
            mainProgram = "mbr";
            platforms = platforms.unix;
          };
        });

      # Clippy check - runs lints without full build
      packages.clippy = craneLib.cargoClippy (commonArgs
        // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets -- -D warnings";

          preBuild = ''
            mkdir -p templates/components-js
            cp -r ${packages.mbr-components}/* templates/components-js/
          '';
        });

      # Format check
      packages.fmt = craneLib.cargoFmt {
        inherit src;
      };

      packages.default = packages.mbr;

      # Checks run by `nix flake check`
      checks = {
        inherit (packages) mbr clippy fmt;
      };

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
      devShells.default = craneLib.devShell (commonEnvVars
        // {
          # Include checks to ensure dev environment matches CI
          checks = self.checks.${system};

          # Build inputs from common + dev tools
          inputsFrom = [packages.mbr];
          packages = with pkgs; [
            cargo-watch
          ]
          ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
            xcodegen  # For generating Xcode project from project.yml
          ]);

          PKG_CONFIG_PATH = "${pkgs.ffmpeg_7-full.dev}/lib/pkgconfig";
          LD_LIBRARY_PATH = "${pkgs.stdenv.cc.cc.lib}/lib";
          RUST_LOG = "mbr=debug,tower_http=debug";

          shellHook = ''
            # Configure git hooks if in a git repo and not already set
            if git rev-parse --git-dir > /dev/null 2>&1; then
              current_hooks_path=$(git config --local core.hooksPath 2>/dev/null || echo "")
              if [[ "$current_hooks_path" != ".githooks" ]]; then
                git config --local core.hooksPath .githooks
                echo "Configured git hooks: core.hooksPath = .githooks"
              fi
            fi
          '';
        });
    });
}
