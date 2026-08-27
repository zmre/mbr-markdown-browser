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
    # Use the darwin-specific channel on macOS (better cached for darwin builds)
    # and the standard nixos channel on Linux. Flake inputs can't be selected
    # conditionally, so both are declared and the right one is picked in outputs.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    nixpkgs-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    nixpkgs-darwin,
    rust-overlay,
    crane,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      overlays = [(import rust-overlay)];
      nixpkgsForSystem =
        if (system == "aarch64-darwin" || system == "x86_64-darwin")
        then nixpkgs-darwin
        else nixpkgs;
      pkgs = import nixpkgsForSystem {
        inherit system overlays;
        config.allowUnfree = true;
      };

      # Get rust toolchain from rust-toolchain.toml. Kept deliberately lean —
      # see the comment in that file for why.
      rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      # Editor/IDE components, added for `nix develop` only.
      #
      # These must NOT be in `rustToolchain`: that one is a runtime reference of
      # every crane derivation, so anything in it is uploaded to the binary cache
      # once per system. rust-analyzer (38 MB) and rust-src (69 MB) are of no use
      # to a compile, and the `profile = "default"` that used to be in
      # rust-toolchain.toml also dragged in rust-docs (695 MB) and llvm-tools
      # (404 MB) — 1.2 GB per system of a 5 GB cache, for an HTML manual and a
      # profiler nothing in this repo invokes.
      rustToolchainDev = rustToolchain.override {
        extensions = ["rust-analyzer" "rust-src"];
      };

      # Create crane lib with our toolchain
      craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

      # Same crane, dev toolchain. Used only by devShells.default, so the extra
      # components never reach a cached build derivation.
      craneLibDev = (crane.mkLib pkgs).overrideToolchain rustToolchainDev;

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
          <!--
            File-type claims. Every entry is Viewer: mbr renders documents, it
            never edits the file it was handed, so it must not advertise an
            editor role.

            Markdown is claimed TWICE on purpose. Since Mac OS X 10.4, a
            CFBundleDocumentTypes dict that contains LSItemContentTypes has its
            legacy CFBundleTypeExtensions / CFBundleTypeMIMETypes /
            CFBundleTypeOSTypes keys IGNORED - the suppression is per-dict, not
            per-bundle. Putting the UTI claim and the extension claim in
            separate dicts is therefore the only way to have both, and the
            extension dict is what still works on a Mac where the markdown UTI
            somehow fails to register.
          -->
          <key>CFBundleDocumentTypes</key>
          <array>
            <dict>
              <key>CFBundleTypeName</key>
              <string>Markdown document</string>
              <key>CFBundleTypeRole</key>
              <string>Viewer</string>
              <!--
                Default (not Alternate): installing MBR makes it the default
                app for markdown files, replacing whatever the user had.
              -->
              <key>LSHandlerRank</key>
              <string>Default</string>
              <key>LSItemContentTypes</key>
              <array>
                <!--
                  Only net.daringfireball.markdown is listed. "public.markdown"
                  is folklore: the "public." namespace is reserved for Apple,
                  and CoreTypes.bundle declares no markdown type of any kind,
                  so shipping it would be shipping an invented UTI.
                -->
                <string>net.daringfireball.markdown</string>
              </array>
            </dict>
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
              <string>Markdown document (by extension)</string>
              <key>CFBundleTypeRole</key>
              <string>Viewer</string>
              <key>LSHandlerRank</key>
              <string>Default</string>
            </dict>
            <dict>
              <key>CFBundleTypeName</key>
              <string>Plain text document</string>
              <key>CFBundleTypeRole</key>
              <string>Viewer</string>
              <!--
                Alternate, deliberately: mbr should show up under "Open With"
                for text files without displacing the user's text editor.

                public.plain-text is a supertype, so this claims more than
                .txt: public.source-code conforms to it, which pulls in ~90
                source extensions (.c, .py, .rb, .sh, .swift, .js, .java, ...)
                plus .log, .csv and .tsv. It does NOT claim .json, .yaml,
                .css, .html or .xml (those conform to public.text directly),
                nor extensions macOS declares no type for (.rs, .toml, .nix).
              -->
              <key>LSHandlerRank</key>
              <string>Alternate</string>
              <key>LSItemContentTypes</key>
              <array>
                <string>public.plain-text</string>
              </array>
            </dict>
          </array>
          <!--
            Without this, the LSItemContentTypes claim above (and the appex's
            QLSupportedContentTypes entry) would match nothing: macOS ships no
            markdown UTI at all, so an unclaimed .md resolves to a dynamic UTI
            that conforms only to public.data. Declaring the type is what binds
            the markdown extensions to net.daringfireball.markdown.

            Imported rather than Exported because mbr does not own this
            identifier - it is Daring Fireball's de-facto community UTI, also
            exported by several markdown editors. Whichever bundle registers
            first wins; the declarations agree, so it does not matter which.
          -->
          <key>UTImportedTypeDeclarations</key>
          <array>
            <dict>
              <key>UTTypeIdentifier</key>
              <string>net.daringfireball.markdown</string>
              <key>UTTypeDescription</key>
              <string>Markdown document</string>
              <key>UTTypeConformsTo</key>
              <array>
                <string>public.plain-text</string>
              </array>
              <key>UTTypeTagSpecification</key>
              <dict>
                <key>public.filename-extension</key>
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
                <key>public.mime-type</key>
                <array>
                  <string>text/markdown</string>
                  <string>text/x-markdown</string>
                </array>
              </dict>
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

      # swiftc target triple for the QuickLook extension. Must track the host
      # arch: the .appex has to match the binary it ships next to or macOS will
      # refuse to load it. Releases are arm64-only now, but x86_64-darwin is
      # still buildable from source, so the Intel branch stays.
      swiftTarget =
        if system == "x86_64-darwin"
        then "x86_64-apple-macos14.0"
        else "arm64-apple-macos14.0";

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
          || (builtins.match ".*/tests/videos/.*" path != null) # Test video fixtures (remux/playability tests)
          || (builtins.match ".*/tests/fixtures/.*" path != null) # Test fixtures (golden HTML, configs)
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
      # EGL fallback for a Nix-built GUI on a non-NixOS host.
      #
      # WebKitGTK 2.52's web process calls `eglGetDisplay` during page creation
      # and `CRASH()`es outright if it comes back `EGL_NO_DISPLAY` -- there is no
      # software path and no environment variable that opts out
      # (WEBKIT_DISABLE_DMABUF_RENDERER and WEBKIT_DISABLE_COMPOSITING_MODE were
      # both tried; the abort is in `initializePlatformDisplayIfNeeded`, upstream
      # of either). So the display has to exist.
      #
      # Nix's libglvnd looks for EGL vendor ICDs in, in order,
      # /run/opengl-driver/share/glvnd/egl_vendor.d, /etc/glvnd/egl_vendor.d and
      # /usr/share/glvnd/egl_vendor.d. On NixOS the first is the system driver
      # and everything works. On Arch (and any other non-NixOS host) only the
      # third exists, and the vendor it names cannot be loaded from a Nix process
      # -- `/usr/lib/libEGL_mesa.so.0` needs the host's `libgallium-*.so`, which
      # is not on a Nix binary's search path and could not safely be put there
      # (it is built against a different glibc). glvnd then finds no vendor at
      # all, `eglGetDisplay` returns EGL_NO_DISPLAY with EGL_BAD_PARAMETER, and
      # the web process aborts before the first paint.
      #
      # So: append nixpkgs' own Mesa as a last-resort vendor. It is *appended*,
      # never substituted -- the host's driver still wins wherever there is one,
      # which keeps NixOS (and nixGL, which exports these same variables) on the
      # exact driver it was going to use.
      #
      # Cost: ~805 MiB of extra closure on a host that has no Mesa in its store,
      # most of it llvm, which every Gallium driver links. On NixOS it is
      # near-zero, because the system already depends on this Mesa.
      glvndVendorDefaults =
        "/run/opengl-driver/share/glvnd/egl_vendor.d:/etc/glvnd/egl_vendor.d:/usr/share/glvnd/egl_vendor.d";
      mesaEglVendorDir = "${pkgs.mesa}/share/glvnd/egl_vendor.d";
      mesaDriDir = "${pkgs.mesa}/lib/dri";

      # Same story one layer down: with an EGL display in hand, WebKit's DMA-BUF
      # renderer asks gbm for buffers, and Nix's mesa-libgbm looks for its backend
      # in `/run/opengl-driver/lib/gbm` alone --
      #
      #   MESA-LOADER: failed to open dri: /run/opengl-driver/lib/gbm/dri_gbm.so
      #
      # after which rendering falls back to a slower path. Not fatal, which is why
      # it only became visible once the abort above was fixed.
      #
      # `GBM_BACKENDS_PATH` is colon-separated (verified: adding the store path
      # after the default silences the message and keeps the host's backend
      # first), so the same append-never-substitute rule applies. That ordering
      # matters more here than for EGL: on a NixOS box with a proprietary driver
      # the host's gbm backend is the *only* correct one.
      mesaGbmDir = "${pkgs.mesa}/lib/gbm";
      gbmBackendsDefault = "/run/opengl-driver/lib/gbm";

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

      # Feature set for the CLI binary and checks.
      # `ffi` (UniFFI/Swift bindings) is only needed for the macOS QuickLook
      # extension, so it's enabled on Darwin only — Linux/Windows builds skip
      # compiling the Swift bindings generator entirely.
      cliFeatures =
        "gui,media-metadata,ffmpeg-static"
        + pkgs.lib.optionalString pkgs.stdenv.isDarwin ",ffi";

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
          cargoExtraArgs = "--locked --features ${cliFeatures}";

          # crane defaults to `zstd -3`; -19 takes this artifact from 942 MB to
          # 582 MB (measured) for byte-identical contents. Worth it because this
          # is the single largest thing we push: 3.54 GB of target/ that is ~63%
          # redundant by construction — `panic = 'abort'` in Cargo.toml plus
          # crane's `cargo test --no-run` means the whole dep tree is compiled
          # twice (libtest needs unwinding), and `cargo check --all-targets`
          # adds a third set of rmeta on top. None of that is safely removable:
          # packages.tests and packages.clippy reuse exactly those artifacts.
          #
          # The cost lands only when this derivation actually rebuilds (a
          # Cargo.lock bump): ~7s -> ~140s of compression. Every consumer job
          # then downloads 360 MB less, so on balance CI gets faster.
          #
          # Deliberately not `--long=27` or higher: crane's inherit hook
          # decompresses with a plain `zstd -d`, which would need a matching
          # window size. -19 needs no such flag.
          zstdCompressionExtraArgs = "-19";
          preBuild = ''
            # crane's mkDummySrc strips `required-features` from every [[bin]]
            # (see crane's cleanCargoToml.nix). That un-gates the `uniffi-bindgen`
            # binary, so a deps-only `cargo build`/`check` builds it unconditionally
            # and pulls in the macOS-only `uniffi` build-dependency even on Linux.
            # Re-add the gate so the bin (and uniffi) is only built when `ffi` is on
            # — enabled on Darwin, off elsewhere via cliFeatures.
            grep -q 'required-features = \["ffi"\]' Cargo.toml \
              || sed -i '/path = "uniffi-bindgen.rs"/a required-features = ["ffi"]' Cargo.toml

            # Create empty component files for dependency resolution
            # Must match the actual file names produced by vite build (see vite.config.ts)
            mkdir -p templates/components-js
            touch templates/components-js/mbr-components.min.js
          '';
        });

      # The exact feature set Windows ships: GUI, no media-metadata, no ffi.
      # Bound once so `packages.clippy-minimal`, `packages.tests-minimal`, and
      # ci.yml's Windows job cannot drift apart silently.
      minimalFeatures = "gui";

      # Dependency artifacts for the minimal feature set.
      #
      # This cannot reuse `cargoArtifacts` above: a different feature set is a
      # different dependency graph (no ffmpeg-next, pdfium-render, metadata, or
      # uniffi), so cargo would rebuild the world anyway. Giving the minimal
      # checks their own buildDepsOnly means the dep tree is content-addressed
      # and lands in the binary cache — it rebuilds when Cargo.lock changes, not
      # on every source change, and clippy-minimal/tests-minimal share it.
      #
      # commonArgs is reused verbatim (ffmpeg/pdfium stay in buildInputs even
      # though this feature set does not link them). They are already built and
      # cached for the other derivations, so pruning them would cost a second
      # ffmpeg closure rather than save one.
      cargoArtifactsMinimal = craneLib.buildDepsOnly (commonArgs
        // {
          src = craneLib.cleanCargoSource ./.;
          pname = "mbr-minimal";
          cargoExtraArgs = "--locked --no-default-features --features ${minimalFeatures}";

          # Same reasoning as cargoArtifacts above. This one is 747 MB at crane's
          # default, and the two together were 1.69 GB per system — over the 5 GB
          # cache tier on their own once multiplied by the three systems CI builds.
          zstdCompressionExtraArgs = "-19";
          preBuild = ''
            # Same crane mkDummySrc workaround as cargoArtifacts above — see the
            # comment there. It matters more here: `ffi` is off in this feature
            # set, so an un-gated uniffi-bindgen bin would pull the macOS-only
            # uniffi build-dependency into a build that never wants it.
            grep -q 'required-features = \["ffi"\]' Cargo.toml \
              || sed -i '/path = "uniffi-bindgen.rs"/a required-features = ["ffi"]' Cargo.toml

            mkdir -p templates/components-js
            touch templates/components-js/mbr-components.min.js
          '';
        });
    in rec {
      # Package set is assembled as a base set merged with darwin-only sets via
      # `// lib.optionalAttrs isDarwin { ... }`. On Linux those merges contribute
      # nothing, so the darwin-only outputs are ABSENT (not empty-attrset-valued),
      # which keeps `nix flake check` happy on Linux.
      packages =
        {
          # Build frontend components first
          mbr-components = pkgs.buildNpmPackage {
            pname = "mbr-components";
            inherit version;
            src = ./components;
            #npmDepsHash = pkgs.lib.fakeHash;
            npmDepsHash = "sha256-P1YF4/8i/R7uHEJi8plOU3qnr3ZuLw2wwh/P49DrXEc=";
            buildPhase = ''
              npm run build
            '';
            installPhase = ''
              mkdir -p $out
              cp -r ../templates/components-js/* $out/
            '';
          };
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
          # QuickLook staticlib: builds libmbr.a without GUI/ffmpeg for sandbox compatibility
          mbr-quicklook-staticlib = craneLib.buildPackage (commonArgs
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
            });

          # QuickLook extension: builds the .appex using swiftc directly
          mbr-quicklook = pkgs.stdenv.mkDerivation {
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
                -target ${swiftTarget} \
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

              # Copy Info.plist to complete the .appex bundle structure.
              #
              # The plist is shared with the Xcode dev project (project.yml),
              # so it holds $(...) build settings that only xcodebuild expands.
              # This build has to expand them itself or the shipped bundle gets
              # a literal "$(PRODUCT_BUNDLE_IDENTIFIER)" as its identifier,
              # which is not a valid bundle ID and is not prefixed by the host
              # app's, so macOS will not load the extension.
              #
              # --replace-fail: if a token is renamed on either side this build
              # fails loudly instead of silently shipping the unexpanded text.
              cp quicklook/MBRPreview/Info.plist build/MBRPreview.appex/Contents/Info.plist
              substituteInPlace build/MBRPreview.appex/Contents/Info.plist \
                --replace-fail '$(PRODUCT_BUNDLE_IDENTIFIER)' 'com.zmre.mbr.MBRPreview' \
                --replace-fail '$(MARKETING_VERSION)' '${version}' \
                --replace-fail '$(CURRENT_PROJECT_VERSION)' '${version}'
            '';

            installPhase = ''
              mkdir -p $out
              cp -R build/MBRPreview.appex $out/
            '';
          };
        }
        // {
          # Core CLI binary (all platforms) - no app bundle, no QuickLook
          # Statically links ffmpeg — no runtime ffmpeg dependency
          mbr-cli = craneLib.buildPackage (commonArgs
            // {
              inherit cargoArtifacts;
              pname = "mbr-cli";
              cargoExtraArgs = "--locked --features ${cliFeatures}";
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
          mbr =
            if pkgs.stdenv.isDarwin
            then
              pkgs.stdenv.mkDerivation {
                pname = "mbr";
                inherit version;

                # No source needed - we're assembling the app bundle around mbr-cli
                dontUnpack = true;

                # makeBinaryWrapper produces a compiled Mach-O wrapper (not a shell
                # script) for the bin/ entry point below.
                nativeBuildInputs = [pkgs.makeBinaryWrapper];

                installPhase = ''
                  # macOS app bundle. The REAL binary lives inside the bundle at
                  # Contents/MacOS/mbr — it is NOT a wrapper that execs out to the
                  # nix-store CLI. macOS derives NSBundle.mainBundle (and thus the
                  # dock icon, app name, and Info.plist identity) from the running
                  # executable's path, read UNRESOLVED via _NSGetExecutablePath. So a
                  # bundle executable that execs a path outside the bundle would lose
                  # the app identity. Keeping the real binary here makes a Finder
                  # launch resolve to MBR.app correctly.
                  mkdir -p $out/Applications/MBR.app/Contents/{MacOS,Frameworks,Resources,PlugIns}

                  cp ${packages.mbr-cli}/bin/mbr $out/Applications/MBR.app/Contents/MacOS/mbr
                  chmod u+w $out/Applications/MBR.app/Contents/MacOS/mbr

                  # Bundle pdfium in Frameworks/. The bundle binary discovers it
                  # relative to the executable (Contents/MacOS/mbr -> ../Frameworks/),
                  # so no PDFIUM_DYNAMIC_LIB_PATH is needed when launched from the
                  # bundle. Release builds also rely on this when Nix store paths are absent.
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
                  # 3. Sign the app bundle (also signs Contents/MacOS/mbr)
                  /usr/bin/codesign --force --sign - $out/Applications/MBR.app

                  # CLI entry point: a thin wrapper that execs the binary INSIDE the
                  # bundle. Because the wrapper execs a path within MBR.app, after the
                  # exec the process's executable path is Contents/MacOS/mbr, so
                  # running `mbr` from the command line inherits the app identity and
                  # gets the proper dock icon. A bare symlink would NOT work here:
                  # macOS reads the exec path unresolved, so it would look for a bundle
                  # around $out/bin and find none. PDFIUM_DYNAMIC_LIB_PATH is set as a
                  # belt-and-suspenders fallback (the Frameworks copy also works).
                  mkdir -p $out/bin
                  makeBinaryWrapper $out/Applications/MBR.app/Contents/MacOS/mbr $out/bin/mbr \
                    --set PDFIUM_DYNAMIC_LIB_PATH "${pkgs.pdfium-binaries}/lib"
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
                    --set PDFIUM_DYNAMIC_LIB_PATH "${pkgs.pdfium-binaries}/lib" \
                    --set-default __EGL_VENDOR_LIBRARY_DIRS "${glvndVendorDefaults}" \
                    --suffix __EGL_VENDOR_LIBRARY_DIRS : "${mesaEglVendorDir}" \
                    --suffix LIBGL_DRIVERS_PATH : "${mesaDriDir}" \
                    --set-default GBM_BACKENDS_PATH "${gbmBackendsDefault}" \
                    --suffix GBM_BACKENDS_PATH : "${mesaGbmDir}"

                  # XDG desktop integration -- the Linux counterpart of MBR.app.
                  #
                  # Without an entry, an application launcher has only the bare
                  # binary to go on, cannot tell a GUI program from a CLI one, and
                  # runs it through a terminal: two windows, a console and the
                  # browser. The entry says `Terminal=false` and the launcher
                  # execs it directly, which is what makes the console go away.
                  #
                  # It only takes effect once mbr is on an XDG data path, so a
                  # bare `nix build` + `./result/bin/mbr` still shows nothing to a
                  # launcher -- `nix profile install` (or a Home Manager package)
                  # is what puts it under a directory in $XDG_DATA_DIRS.
                  install -Dm644 ${./linux/mbr.desktop} \
                    $out/share/applications/mbr.desktop

                  # 256x256 is the icon's real size; naming the directory
                  # honestly lets the theme scale from it rather than up from a
                  # wrong-sized "512x512".
                  install -Dm644 ${./mbr-icon.png} \
                    $out/share/icons/hicolor/256x256/apps/mbr.png
                '';
                meta = packages.mbr-cli.meta;
              };

          # Clippy check - runs lints without full build
          clippy = craneLib.cargoClippy (commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- -D warnings";

              preBuild = ''
                mkdir -p templates/components-js
                cp -r ${packages.mbr-components}/* templates/components-js/
              '';
            });

          # Test - runs all tests
          tests = craneLib.cargoTest (commonArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "--features ${cliFeatures}";

              preBuild = ''
                mkdir -p templates/components-js
                cp -r ${packages.mbr-components}/* templates/components-js/
              '';
            });

          # Lint + test the exact feature set Windows ships, but on the Nix
          # builder.
          #
          # Cargo feature combinations are only as tested as the combinations you
          # build. `--no-default-features --features gui` turns off
          # `media-metadata`, which cfg-gates whole CLI arguments — and a stale
          # `conflicts_with_all` reference to a gated argument makes clap panic at
          # startup on *every* invocation, not just in tests. That is a
          # feature-resolution bug, not a platform bug, so catching it here gives
          # fast feedback instead of waiting on a Windows runner.
          #
          # These exist as flake attrs rather than `nix develop --command cargo
          # ...` in CI so the compile is cached like every other check: the dev
          # shell was cached, but the cargo build inside it was not, so ci.yml's
          # minimal-features job recompiled this whole dependency tree on every
          # PR.
          clippy-minimal = craneLib.cargoClippy (commonArgs
            // {
              cargoArtifacts = cargoArtifactsMinimal;
              pname = "mbr-minimal";
              cargoExtraArgs = "--locked --no-default-features --features ${minimalFeatures}";
              cargoClippyExtraArgs = "--all-targets -- -D warnings";

              preBuild = ''
                mkdir -p templates/components-js
                cp -r ${packages.mbr-components}/* templates/components-js/
              '';
            });

          tests-minimal = craneLib.cargoTest (commonArgs
            // {
              cargoArtifacts = cargoArtifactsMinimal;
              pname = "mbr-minimal";
              # Features go in cargoExtraArgs ONLY. crane composes the command as
              # `cargo test ${cargoExtraArgs} ${cargoTestExtraArgs}`, and cargo
              # rejects a repeated `--no-default-features` ("cannot be used
              # multiple times"), so setting the feature flags in both places
              # fails the build. The sibling `tests` attr above puts them in
              # cargoTestExtraArgs instead, which is fine there only because it
              # passes `--features` alone.
              cargoExtraArgs = "--locked --no-default-features --features ${minimalFeatures}";

              preBuild = ''
                mkdir -p templates/components-js
                cp -r ${packages.mbr-components}/* templates/components-js/
              '';
            });

          # Format check
          fmt = craneLib.cargoFmt {
            inherit src;
          };
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
          # Swift format check (Darwin only)
          # Excludes Generated/ directory (UniFFI auto-generated code)
          swiftfmt =
            pkgs.runCommand "mbr-swiftfmt-check" {
              nativeBuildInputs = [pkgs.swiftformat];
            } ''
              cd ${src}/quicklook
              # Use explicit exclusion since config file may not be accessible in sandbox
              swiftformat --lint --swiftversion 5.9 --exclude Generated . 2>&1 || (echo "Swift formatting check failed" && exit 1)
              touch $out
            '';

          # Swift lint check (Darwin only)
          # Excludes Generated/ directory (UniFFI auto-generated code)
          swiftlint-check =
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
            '';
        }
        // {
          # Expose the minimal static ffmpeg for independent build/verification.
          #
          # Both of these are also named directly by ci.yml's "Push and pin
          # consumer artifacts" step. They are fixed-version, effectively
          # immutable, and cost 30-60 minutes to rebuild from source, so they are
          # pinned in the binary cache against LRU eviction — which requires them
          # to be addressable as flake attrs. x264Static is pinned in its own
          # right rather than as a reference of ffmpegMinimalStatic: the Cachix
          # docs promise only that "pinned paths are immune from garbage
          # collection", never that the promise extends to the closure.
          ffmpegMinimalStatic = ffmpegMinimalStatic;
          x264Static = x264Static;

          default = packages.mbr;

          # Release package: creates distributable archives from the built package
          # Bundles pdfium library for PDF cover image generation
          release =
            pkgs.runCommand "mbr-release-${version}" {
              nativeBuildInputs = [pkgs.gnutar pkgs.gzip];
            } (
              if pkgs.stdenv.isDarwin
              then ''
                mkdir -p $out

                # --- /nix/store independence -------------------------------
                #
                # Release archives must run on a Mac that has never had Nix
                # installed, so no Mach-O in them may reference /nix/store.
                #
                # Two failure modes made the previous approach fragile, and
                # both fail SILENTLY (green build, broken download):
                #
                #   1. `install_name_tool -change OLD NEW` is a no-op when OLD
                #      does not match exactly. If pkgs.libiconv ever resolves
                #      to a different store path than the one actually linked,
                #      the rewrite quietly does nothing.
                #   2. It was applied to a hardcoded list of two files, so any
                #      newly bundled Mach-O (another dylib in Frameworks/, a
                #      second plug-in) was never covered.
                #
                # So: discover Mach-Os instead of listing them, and afterwards
                # hard-fail if any store reference survives. `otool -L` prints
                # dependencies as tab-indented lines and prints none at all for
                # a non-Mach-O, so it doubles as the file-type test.

                machoDeps() {
                  /usr/bin/otool -L "$1" 2>/dev/null | grep '^	' | awk '{print $1}'
                }

                machoRpaths() {
                  /usr/bin/otool -l "$1" 2>/dev/null \
                    | awk '/LC_RPATH/{r=1} r && /^ *path /{print $2; r=0}'
                }

                # Rewrite store dylib references to their macOS system twins.
                portablize() {
                  local f dep
                  for f in $(find "$1" -type f); do
                    for dep in $(machoDeps "$f" | grep '^/nix/store/'); do
                      case "$(basename "$dep")" in
                        libiconv.2.dylib)
                          # macOS ships libiconv; Nix's copy is build-time only.
                          chmod u+w "$f"
                          /usr/bin/install_name_tool -change \
                            "$dep" /usr/lib/libiconv.2.dylib "$f"
                          ;;
                        *)
                          # Deliberately not auto-mapped to /usr/lib/<name>: a
                          # wrong guess swaps a loud failure for a silent one.
                          # New store deps must be handled consciously here.
                          # The audit below turns this into a build failure.
                          ;;
                      esac
                    done
                  done
                }

                # Fail the build if ANY Mach-O still resolves into the store.
                auditPortable() {
                  local f bad refs
                  bad=0
                  for f in $(find "$1" -type f); do
                    refs="$( (machoDeps "$f"; machoRpaths "$f") | grep '^/nix/store/' || true)"
                    if [ -n "$refs" ]; then
                      echo "error: $f references /nix/store:" >&2
                      echo "$refs" >&2
                      bad=1
                    fi
                  done
                  if [ "$bad" -ne 0 ]; then
                    echo "error: release artifacts are not portable" >&2
                    exit 1
                  fi
                }

                # Create staging directory for app bundle
                # Start from the full app bundle (has pdfium, QuickLook, etc.)
                mkdir -p staging
                cp -R ${packages.mbr}/Applications/MBR.app staging/

                # Replace the wrapper binary with the unwrapped binary (no pdfium env var wrapper)
                # The release bundle has pdfium in Frameworks/ so the wrapper is unnecessary
                chmod u+w staging/MBR.app/Contents/MacOS
                chmod u+w staging/MBR.app/Contents/MacOS/mbr
                cp ${packages.mbr-cli}/bin/mbr staging/MBR.app/Contents/MacOS/mbr

                # Rewrite store dylib references across EVERY Mach-O in the
                # bundle (main binary, Frameworks/, PlugIns/*.appex), then
                # prove none survived. Must run before codesign below, since
                # install_name_tool invalidates any signature it touches.
                portablize staging
                auditPortable staging

                # Re-sign: replacing the binary invalidates the original signature.
                # codesign may fail inside Nix sandbox, so allow failure and strip
                # invalid signatures if signing doesn't work.
                # NEVER fall back to `codesign --remove-signature` here. It was
                # doing active harm: codesign cannot reach its daemon inside the
                # Nix sandbox, so the fallback always ran, and it stripped the
                # ad-hoc signature that the linker (and install_name_tool)
                # applies automatically. On Apple Silicon an unsigned Mach-O is
                # SIGKILLed by the kernel, so the tarball shipped an app that
                # could not launch at all -- `codesign --verify` reported "code
                # object is not signed at all" and running it exited 137.
                #
                # Leaving the ad-hoc signature in place is strictly better: it
                # is what makes the binary runnable, and scripts/make-macos-dmg.sh
                # re-signs properly on the CI runner (outside the sandbox, where
                # codesign works) before packaging the DMG.
                /usr/bin/codesign --force --sign - \
                  staging/MBR.app/Contents/Frameworks/libpdfium.dylib 2>/dev/null || true
                /usr/bin/codesign --force --sign - \
                  --entitlements ${./quicklook/MBRPreview/MBRPreview.entitlements} \
                  staging/MBR.app/Contents/PlugIns/MBRPreview.appex 2>/dev/null || true
                /usr/bin/codesign --force --sign - staging/MBR.app 2>/dev/null || true

                # An unsigned Mach-O cannot run on Apple Silicon, so treat a
                # missing signature as a build failure rather than shipping a
                # download that dies with SIGKILL.
                for f in staging/MBR.app/Contents/MacOS/mbr \
                         staging/MBR.app/Contents/Frameworks/libpdfium.dylib; do
                  if /usr/bin/codesign -dv "$f" 2>&1 | grep -q "not signed"; then
                    echo "error: $f is unsigned; it would be SIGKILLed on arm64" >&2
                    exit 1
                  fi
                done

                # Create .app bundle archive
                tar -czvf $out/mbr-${archString}.tar.gz \
                  -C staging \
                  MBR.app

                # Create CLI archive with bundled pdfium in lib/ subdirectory
                mkdir -p staging-cli/lib
                cp ${packages.mbr-cli}/bin/mbr staging-cli/
                cp ${pkgs.pdfium-binaries}/lib/libpdfium.dylib staging-cli/lib/
                # Same treatment for the CLI archive: rewrite, then prove it.
                portablize staging-cli
                auditPortable staging-cli
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
        };

      # Checks run by `nix flake check`
      checks =
        {
          inherit (packages) mbr-cli clippy fmt tests clippy-minimal tests-minimal;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
          inherit (packages) swiftfmt swiftlint-check mbr;
        };

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

      # Development shell.
      #
      # craneLibDev, not craneLib: this is the one place rust-analyzer and
      # rust-src belong. Keeping them out of craneLib is what keeps them out of
      # every cached build derivation's closure.
      devShells.default = craneLibDev.devShell (commonEnvVars
        // (pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          # `cargo run -- -g` inside this shell links the *Nix* WebKitGTK from
          # `inputsFrom`, so it hits the same missing-EGL abort the packaged
          # binary does and needs the same fallback. See `mesaEglVendorDir`.
          __EGL_VENDOR_LIBRARY_DIRS = "${glvndVendorDefaults}:${mesaEglVendorDir}";
          LIBGL_DRIVERS_PATH = mesaDriDir;
          GBM_BACKENDS_PATH = "${gbmBackendsDefault}:${mesaGbmDir}";
        })
        // {
          # Include checks to ensure dev environment matches CI
          checks = self.checks.${system};

          # Build inputs from common + dev tools
          inputsFrom = [packages.mbr];
          packages = with pkgs;
            [
              cargo-watch
              cargo-audit
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
