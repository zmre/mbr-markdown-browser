# flake.nix
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
          || (builtins.match ".*/tests/pdfs/.*\\.pdf$" path != null) # Test PDF files
          # QuickLook extension sources
          || (builtins.match ".*\\.swift$" path != null)
          || (builtins.match ".*\\.plist$" path != null)
          || (builtins.match ".*\\.entitlements$" path != null)
          || (builtins.match ".*\\.modulemap$" path != null)
          || (builtins.match ".*\\.h$" path != null) # C headers for FFI
          || (builtins.match ".*/quicklook/project\\.yml$" path != null)
          || (builtins.match ".*/quicklook/build\\.sh$" path != null)
          # Swift tooling config
          || (builtins.match ".*/quicklook/\\.swiftformat$" path != null)
          || (builtins.match ".*/quicklook/\\.swiftlint\\.yml$" path != null);
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

      # Shared build inputs — all builds use static ffmpeg (no runtime ffmpeg dependency)
      commonBuildInputs = with pkgs;
        [
          ffmpegMinimalStatic
          pdfium-binaries
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

      # Static x264 for H.264 software encoding fallback
      # Zero dependencies — only libc. ~3MB added to binary.
      # nixpkgs' x264 only provides dynamic libs, so we build our own static lib.
      # Source, rev, and patches match nixpkgs' x264 package for consistency.
      x264Static = pkgs.stdenv.mkDerivation {
        pname = "x264-static";
        version = "unstable-2025-01-03";
        src = pkgs.fetchFromGitLab {
          domain = "code.videolan.org";
          owner = "videolan";
          repo = "x264";
          rev = "373697b467f7cd0af88f1e9e32d4f10540df4687";
          hash = "sha256-WWtS/UfKA4i1yakHErUnyT/3/+Wy2H5F0U0CmxW4ick=";
        };
        # nasm only needed on x86; ARM uses .S files assembled by $CC
        nativeBuildInputs =
          pkgs.lib.optional pkgs.stdenv.hostPlatform.isx86 pkgs.nasm
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [pkgs.apple-sdk];
        enableParallelBuilding = true;
        configurePlatforms = [];
        # Match nixpkgs: on x86 unset AS (use nasm), on ARM set AS=$CC
        # so .S assembly files go through the C preprocessor
        preConfigure =
          pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isx86 ''
            unset AS
          ''
          + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isAarch ''
            export AS=$CC
          '';
        configureFlags = [
          "--enable-static"
          "--disable-shared"
          "--enable-pic"
          "--disable-cli"
        ];
      };

      # Minimal static ffmpeg used by all builds
      # Zero external codec dependencies — only system frameworks + libc + libx264
      # Static linking avoids hardcoded Nix store paths for ffmpeg dylibs in binaries
      ffmpegMinimalStatic = pkgs.stdenv.mkDerivation {
        pname = "ffmpeg-minimal-static";
        version = "7.1";
        src = pkgs.fetchurl {
          url = "https://ffmpeg.org/releases/ffmpeg-7.1.tar.xz";
          hash = "sha256-QJc9RJcNvIPvMCsGCfLnSYK+LYWRbdLudHLTBninq+Y=";
        };
        unpackCmd = "tar xf $curSrc";
        sourceRoot = "ffmpeg-7.1";
        nativeBuildInputs = with pkgs;
          [pkg-config perl yasm nasm]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [pkgs.apple-sdk];
        buildInputs = [x264Static];

        configurePhase = ''
          ./configure \
            --prefix=$out \
            --cc=$CC --cxx=$CXX \
            --enable-static --disable-shared --enable-pic \
            --disable-autodetect --disable-programs --disable-doc \
            --enable-gpl --enable-version3 \
            --enable-avcodec --enable-avformat --enable-avfilter \
            --enable-avdevice --enable-swscale --enable-swresample \
            --enable-libx264 \
            ${pkgs.lib.optionalString pkgs.stdenv.isDarwin
            "--enable-videotoolbox --enable-audiotoolbox"} \
            --extra-cflags="-w -O3"
        '';
        buildPhase = "make -j$NIX_BUILD_CORES";
        installPhase = "make install";
      };

      # Shared environment variables for builds
      # All builds use static ffmpeg — no FFMPEG_DIR (which forces build.rs to skip
      # pkg-config). Instead, PKG_CONFIG_PATH lets ffmpeg-sys-next discover our static libs.
      commonEnvVars = {
        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        PKG_CONFIG_PATH = "${ffmpegMinimalStatic}/lib/pkgconfig";
        PDFIUM_DYNAMIC_LIB_PATH = "${pkgs.pdfium-binaries}/lib";
        # Tell bindgen where to find glibc headers on Linux (required by ffmpeg-sys-next)
        BINDGEN_EXTRA_CLANG_ARGS =
          pkgs.lib.optionalString pkgs.stdenv.isLinux
          "-isystem ${pkgs.stdenv.cc.libc.dev}/include";
        # ffmpeg-sys-next's build.rs unconditionally links deprecated macOS frameworks
        # (QTKit, OpenGL, VideoDecodeAcceleration) when static linking is enabled.
        # Our minimal ffmpeg doesn't use them, but they fail to load on macOS 15+.
        # Use -weak_framework so dyld doesn't fail if they're absent at runtime.
        # RUSTDOCFLAGS is needed too: doc-tests are compiled by rustdoc (not rustc),
        # so RUSTFLAGS alone doesn't cover them.
        RUSTFLAGS =
          pkgs.lib.optionalString pkgs.stdenv.isDarwin
          (builtins.concatStringsSep " " (map (f: "-C link-arg=-Wl,-weak_framework,${f}") [
            "QTKit"
            "OpenGL"
            "VideoDecodeAcceleration"
          ]));
        RUSTDOCFLAGS =
          pkgs.lib.optionalString pkgs.stdenv.isDarwin
          (builtins.concatStringsSep " " (map (f: "-C link-arg=-Wl,-weak_framework,${f}") [
            "QTKit"
            "OpenGL"
            "VideoDecodeAcceleration"
          ]));
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
          cargoExtraArgs = "--locked --features gui,media-metadata,ffmpeg-static,ffi";
          preBuild = ''
            # Create empty component files for dependency resolution
            # Must match the actual file names produced by vite build (see vite.config.ts)
            mkdir -p templates/components-js
            touch templates/components-js/mbr-components.min.js
          '';
        });
    in rec {
      # Build frontend components first
      packages.mbr-components = pkgs.buildNpmPackage {
        pname = "mbr-components";
        inherit version;
        src = ./components;
        #npmDepsHash = pkgs.lib.fakeHash;
        npmDepsHash = "sha256-QgTHc3tFhpAEQiR/uD+ry2HS/qLyVp4pc8vFpjdkQqY=";
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

      # Core CLI binary (all platforms) - no app bundle, no QuickLook
      # Statically links ffmpeg — no runtime ffmpeg dependency
      packages.mbr-cli = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          pname = "mbr-cli";
          cargoExtraArgs = "--locked --features gui,media-metadata,ffmpeg-static,ffi";
          doCheck = false; # Tests run separately via packages.tests

          preBuild = ''
            mkdir -p templates/components-js
            cp -r ${packages.mbr-components}/* templates/components-js/
          '';

          meta = with pkgs.lib; {
            description = "A markdown viewer, browser, and static site generator (CLI only)";
            homepage = "https://github.com/zmre/mbr";
            license = licenses.gpl3Plus;
            mainProgram = "mbr";
            platforms = platforms.unix;
          };
        });

      # Main package: CLI on Linux, CLI + app bundle + QuickLook on macOS
      packages.mbr =
        if pkgs.stdenv.isDarwin
        then
          pkgs.stdenv.mkDerivation {
            pname = "mbr";
            inherit version;

            # No source needed - we're wrapping mbr-cli
            dontUnpack = true;

            # Use makeBinaryWrapper for macOS - shell scripts can't be CFBundleExecutable
            # for properly signed apps (Gatekeeper issues, code signing problems)
            nativeBuildInputs = [pkgs.makeBinaryWrapper];

            installPhase = ''
              # CLI binary accessible at $out/bin/mbr (wrapped with pdfium path)
              mkdir -p $out/bin
              makeBinaryWrapper ${packages.mbr-cli}/bin/mbr $out/bin/mbr \
                --set PDFIUM_DYNAMIC_LIB_PATH "${pkgs.pdfium-binaries}/lib"

              # macOS app bundle
              mkdir -p $out/Applications/MBR.app/Contents/{MacOS,Frameworks,Resources,PlugIns}

              # Use makeBinaryWrapper for app bundle executable (creates compiled binary, not shell script)
              # This is required for proper code signing and Gatekeeper compatibility
              makeBinaryWrapper ${packages.mbr-cli}/bin/mbr \
                $out/Applications/MBR.app/Contents/MacOS/mbr \
                --set PDFIUM_DYNAMIC_LIB_PATH "${pkgs.pdfium-binaries}/lib"

              # Bundle pdfium in Frameworks/ for fallback (release builds need this when
              # Nix store paths don't exist; Rust code searches Contents/Frameworks/)
              cp ${pkgs.pdfium-binaries}/lib/libpdfium.dylib $out/Applications/MBR.app/Contents/Frameworks/

              cp ${infoPlist} $out/Applications/MBR.app/Contents/Info.plist
              cp ${./macos/AppIcon.icns} $out/Applications/MBR.app/Contents/Resources/AppIcon.icns

              # QuickLook extension (make writable for codesigning)
              cp -R ${packages.mbr-quicklook}/MBRPreview.appex $out/Applications/MBR.app/Contents/PlugIns/
              chmod -R u+w $out/Applications/MBR.app/Contents/PlugIns/MBRPreview.appex

              # Sign components from innermost to outermost:
              # 1. Sign the bundled framework library
              /usr/bin/codesign --force --sign - \
                $out/Applications/MBR.app/Contents/Frameworks/libpdfium.dylib
              # 2. Sign the QuickLook extension with its entitlements
              /usr/bin/codesign --force --sign - \
                --entitlements ${./quicklook/MBRPreview/MBRPreview.entitlements} \
                $out/Applications/MBR.app/Contents/PlugIns/MBRPreview.appex
              # 3. Sign the app bundle
              /usr/bin/codesign --force --sign - $out/Applications/MBR.app
            '';

            meta = with pkgs.lib; {
              description = "A markdown viewer, browser, and static site generator";
              homepage = "https://github.com/zmre/mbr";
              license = licenses.gpl3Plus;
              mainProgram = "mbr";
              platforms = platforms.darwin;
            };
          }
        else
          # Linux: wrap the CLI binary with pdfium path
          pkgs.stdenv.mkDerivation {
            pname = "mbr";
            inherit version;
            dontUnpack = true;
            nativeBuildInputs = [pkgs.makeBinaryWrapper];
            installPhase = ''
              mkdir -p $out/bin
              makeBinaryWrapper ${packages.mbr-cli}/bin/mbr $out/bin/mbr \
                --set PDFIUM_DYNAMIC_LIB_PATH "${pkgs.pdfium-binaries}/lib"
            '';
            meta = packages.mbr-cli.meta;
          };

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

      # Test - runs all tests
      packages.tests = craneLib.cargoTest (commonArgs
        // {
          inherit cargoArtifacts;
          cargoTestExtraArgs = "--features gui,media-metadata,ffmpeg-static,ffi";

          preBuild = ''
            mkdir -p templates/components-js
            cp -r ${packages.mbr-components}/* templates/components-js/
          '';
        });

      # Format check
      packages.fmt = craneLib.cargoFmt {
        inherit src;
      };

      # Swift format check (Darwin only)
      # Excludes Generated/ directory (UniFFI auto-generated code)
      packages.swiftfmt = pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin (
        pkgs.runCommand "mbr-swiftfmt-check" {
          nativeBuildInputs = [pkgs.swiftformat];
        } ''
          cd ${src}/quicklook
          # Use explicit exclusion since config file may not be accessible in sandbox
          swiftformat --lint --swiftversion 5.9 --exclude Generated . 2>&1 || (echo "Swift formatting check failed" && exit 1)
          touch $out
        ''
      );

      # Swift lint check (Darwin only)
      # Excludes Generated/ directory (UniFFI auto-generated code)
      packages.swiftlint-check = pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin (
        pkgs.runCommand "mbr-swiftlint-check" {
          nativeBuildInputs = [pkgs.swiftlint];
          # SwiftLint needs HOME for cache directory
          HOME = "/tmp";
        } ''
          cd ${src}/quicklook
          # Check for violations (swiftlint may error about cache but still report correctly)
          output=$(swiftlint lint --config .swiftlint.yml . 2>&1 || true)
          echo "$output"
          # Fail if violations found
          if echo "$output" | grep -q "Found [1-9][0-9]* violation"; then
            echo "SwiftLint check failed - violations found"
            exit 1
          fi
          touch $out
        ''
      );

      # Expose the minimal static ffmpeg for independent build/verification
      packages.ffmpegMinimalStatic = ffmpegMinimalStatic;

      packages.default = packages.mbr;

      # Checks run by `nix flake check`
      checks =
        {
          inherit (packages) mbr-cli clippy fmt tests;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
          inherit (packages) swiftfmt swiftlint-check mbr;
        };

      # Release package: creates distributable archives from the built package
      # Bundles pdfium library for PDF cover image generation
      packages.release =
        pkgs.runCommand "mbr-release-${version}" {
          nativeBuildInputs = [pkgs.gnutar pkgs.gzip];
        } (
          if pkgs.stdenv.isDarwin
          then ''
            mkdir -p $out

            # Create staging directory for app bundle
            # Start from the full app bundle (has pdfium, QuickLook, etc.)
            mkdir -p staging
            cp -R ${packages.mbr}/Applications/MBR.app staging/

            # Replace the wrapper binary with the unwrapped binary (no pdfium env var wrapper)
            # The release bundle has pdfium in Frameworks/ so the wrapper is unnecessary
            chmod u+w staging/MBR.app/Contents/MacOS
            chmod u+w staging/MBR.app/Contents/MacOS/mbr
            cp ${packages.mbr-cli}/bin/mbr staging/MBR.app/Contents/MacOS/mbr

            # Rewrite Nix store libiconv path to system libiconv
            # Nix's linker uses its own libiconv, but macOS ships /usr/lib/libiconv.2.dylib
            /usr/bin/install_name_tool -change \
              ${pkgs.libiconv}/lib/libiconv.2.dylib \
              /usr/lib/libiconv.2.dylib \
              staging/MBR.app/Contents/MacOS/mbr

            # Re-sign: replacing the binary invalidates the original signature.
            # codesign may fail inside Nix sandbox, so allow failure and strip
            # invalid signatures if signing doesn't work.
            /usr/bin/codesign --force --sign - \
              staging/MBR.app/Contents/Frameworks/libpdfium.dylib 2>/dev/null || \
              /usr/bin/codesign --remove-signature \
                staging/MBR.app/Contents/Frameworks/libpdfium.dylib 2>/dev/null || true
            /usr/bin/codesign --force --sign - \
              --entitlements ${./quicklook/MBRPreview/MBRPreview.entitlements} \
              staging/MBR.app/Contents/PlugIns/MBRPreview.appex 2>/dev/null || \
              /usr/bin/codesign --remove-signature \
                staging/MBR.app/Contents/PlugIns/MBRPreview.appex 2>/dev/null || true
            /usr/bin/codesign --force --sign - staging/MBR.app 2>/dev/null || \
              /usr/bin/codesign --remove-signature staging/MBR.app 2>/dev/null || true

            # Create .app bundle archive
            tar -czvf $out/mbr-${archString}.tar.gz \
              -C staging \
              MBR.app

            # Create CLI archive with bundled pdfium in lib/ subdirectory
            mkdir -p staging-cli/lib
            cp ${packages.mbr-cli}/bin/mbr staging-cli/
            cp ${pkgs.pdfium-binaries}/lib/libpdfium.dylib staging-cli/lib/
            # Rewrite Nix store libiconv path to system libiconv
            /usr/bin/install_name_tool -change \
              ${pkgs.libiconv}/lib/libiconv.2.dylib \
              /usr/lib/libiconv.2.dylib \
              staging-cli/mbr
            tar -czvf $out/mbr-cli-${archString}.tar.gz \
              -C staging-cli \
              mbr lib

            # Create checksums
            cd $out
            sha256sum *.tar.gz > SHA256SUMS

            echo ""
            echo "Release artifacts:"
            ls -lh $out/
          ''
          else ''
            mkdir -p $out

            # Create CLI archive with bundled pdfium in lib/ subdirectory
            mkdir -p staging/lib
            cp ${packages.mbr-cli}/bin/mbr staging/
            cp ${pkgs.pdfium-binaries}/lib/libpdfium.so staging/lib/
            tar -czvf $out/mbr-${archString}.tar.gz \
              -C staging \
              mbr lib

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
          packages = with pkgs;
            [
              cargo-watch
              imagemagick
            ]
            ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
              xcodegen # For generating Xcode project from project.yml
              swiftformat # Swift code formatter (like cargo fmt)
              swiftlint # Swift linter (like cargo clippy)
            ]);

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
