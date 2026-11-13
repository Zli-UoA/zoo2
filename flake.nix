{
  description = "Zoo2 - Zli cOmmunication tOol";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix/monthly";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, fenix, flake-utils, nixpkgs, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-zm3dyIY2T414ZRR3EhLOvptzG6gta4WZUcawzMUWtqI=";
        };
        pkgs = nixpkgs.legacyPackages.${system};
        rustPlatform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };
        zoo2-tui = rustPlatform.buildRustPackage rec {
          pname = "zoo2-tui";
          version = (builtins.fromTOML (builtins.readFile ./tui/Cargo.toml)).package.version;
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./client-core
              ./client-types
              ./tui
            ];
          };
          sourceRoot = "${src.name}/tui";
          cargoLock.lockFile = ./tui/Cargo.lock;
          # テストは CI TUI で実行するので、ビルドだけにする。
          doCheck = false;
          meta = {
            description = "zoo2 をターミナルから使うチャットクライアント";
            homepage = "https://github.com/Zli-UoA/zoo2/tree/main/tui";
            license = pkgs.lib.licenses.mit;
            mainProgram = "zoo2-tui";
          };
        };
        linuxDesktopLibs = with pkgs; [
          alsa-lib
          fontconfig
          freetype
          libx11
          libxcb
          libxcursor
          libxi
          libxkbcommon
          libxrandr
          vulkan-loader
          wayland
        ];
        # デスクトップの画面撮影 (scripts/desktop-screenshot.sh) 用。
        # 仮想ディスプレイ (Xvfb) と WM (openbox) の上で GPUI を lavapipe (CPU の Vulkan) で描く。
        desktopScreenshotTools = with pkgs; [
          xvfb
          openbox
          xwd
          imagemagick
        ];
        # Tauri 版（desktop-tauri）の WebKitGTK と、そのランタイムで読まれるもの。
        # libayatana-appindicator はトレイのために dlopen されるので LD_LIBRARY_PATH にも入れる。
        tauriLibs = with pkgs; [
          webkitgtk_4_1
          libsoup_3
          gtk3
          glib
          glib-networking
          librsvg
          cairo
          pango
          gdk-pixbuf
          atk
          dbus
          openssl
          libayatana-appindicator
          gst_all_1.gstreamer
          gst_all_1.gst-plugins-base
          gst_all_1.gst-plugins-good
          gst_all_1.gst-plugins-bad
        ];
        # デスクトップ版（desktop-tauri）。同梱の画面（Leptos の WASM）を先に作り、Rust 側に埋め込む。
        # Nix で入れた版は自分では更新しない（ストアは書き換えられず、bundle_type() も取れないため）。
        # 更新は flake の入力を上げて行う（desktop-tauri/README.md）。
        desktopTauriVersion = (builtins.fromTOML (builtins.readFile ./desktop-tauri/src-tauri/Cargo.toml)).package.version;
        desktopTauriSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./client-core
            ./client-types
            (pkgs.lib.fileset.difference ./desktop-tauri (pkgs.lib.fileset.unions [
              (pkgs.lib.fileset.maybeMissing ./desktop-tauri/ui/dist)
              (pkgs.lib.fileset.maybeMissing ./desktop-tauri/ui/target)
              (pkgs.lib.fileset.maybeMissing ./desktop-tauri/src-tauri/target)
              (pkgs.lib.fileset.maybeMissing ./desktop-tauri/src-tauri/gen)
            ]))
          ];
        };
        # 画面のビルドは長いので、Rust 側の変更で作り直さないよう、ソースを画面の分だけにする。
        desktopTauriUiSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./client-types
            (pkgs.lib.fileset.difference ./desktop-tauri/ui (pkgs.lib.fileset.unions [
              (pkgs.lib.fileset.maybeMissing ./desktop-tauri/ui/dist)
              (pkgs.lib.fileset.maybeMissing ./desktop-tauri/ui/target)
            ]))
          ];
        };
        zoo2-desktop-tauri-ui = rustPlatform.buildRustPackage {
          pname = "zoo2-desktop-tauri-ui";
          version = desktopTauriVersion;
          src = desktopTauriUiSrc;
          sourceRoot = "${desktopTauriUiSrc.name}/desktop-tauri/ui";
          cargoLock.lockFile = ./desktop-tauri/ui/Cargo.lock;
          # wasm-bindgen は desktop-tauri/ui/Cargo.toml で固定した版と同じもの。
          # trunk は --release で wasm-opt (binaryen) も使う。オフラインなので取りに行けない。
          nativeBuildInputs = with pkgs; [ trunk wasm-bindgen-cli binaryen ];
          buildPhase = ''
            runHook preBuild
            trunk build --release --offline
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            cp -r dist $out
            runHook postInstall
          '';
          doCheck = false;
        };
        zoo2-desktop-tauri = rustPlatform.buildRustPackage {
          pname = "zoo2-desktop-tauri";
          version = desktopTauriVersion;
          src = desktopTauriSrc;
          sourceRoot = "${desktopTauriSrc.name}/desktop-tauri/src-tauri";
          cargoLock.lockFile = ./desktop-tauri/src-tauri/Cargo.lock;
          # tauri::generate_context! が ../ui/dist を埋め込む。
          preBuild = ''
            # 展開したソースは読み取り専用なので、書き込めるようにしてから置く。
            chmod u+w ..
            mkdir -p ../ui
            chmod u+w ../ui
            rm -rf ../ui/dist
            cp -r ${zoo2-desktop-tauri-ui} ../ui/dist
            chmod -R u+w ../ui/dist
          '';
          # cargo tauri build と同じく、埋め込んだ画面を custom protocol で配る（devUrl を使わない）。
          buildFeatures = [ "tauri/custom-protocol" ];
          # リンクした WebKitGTK などを RUNPATH に入れる (入れないと起動時に見つからない)。
          nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook3 copyDesktopItems autoPatchelfHook ];
          buildInputs = tauriLibs;
          # トレイの libayatana-appindicator は dlopen で読まれる。
          preFixup = ''
            gappsWrapperArgs+=(--prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ]}")
          '';
          postInstall = ''
            for size in 32x32 64x64 128x128; do
              install -Dm644 icons/$size.png $out/share/icons/hicolor/$size/apps/zoo2-desktop-tauri.png
            done
            install -Dm644 icons/128x128@2x.png $out/share/icons/hicolor/256x256/apps/zoo2-desktop-tauri.png
          '';
          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "zoo2-desktop-tauri";
              desktopName = "zoo2";
              comment = "zoo2 のデスクトップクライアント";
              exec = "zoo2-desktop-tauri";
              icon = "zoo2-desktop-tauri";
              categories = [ "Network" "Chat" ];
              startupWMClass = "zoo2-desktop-tauri";
            })
          ];
          # テストは CI Desktop Tauri で実行するので、ビルドだけにする。
          doCheck = false;
          meta = {
            description = "zoo2 のデスクトップクライアント (Tauri)";
            homepage = "https://github.com/Zli-UoA/zoo2/tree/main/desktop-tauri";
            license = pkgs.lib.licenses.mit;
            mainProgram = "zoo2-desktop-tauri";
            platforms = pkgs.lib.platforms.linux;
          };
        };
        # Android（frontend-compose）の zoo-crypto のバインディング（crates/zoo-crypto-ffi、#353）。
        # Android SDK と NDK のライセンスに同意したうえで使う。既定の devShell には入れず、
        # `nix develop .#android` だけで使う（SDK と NDK は大きいので、Android を触らない人には入れない）。
        androidPkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        };
        # CI Android（.github/workflows/ci-android.yaml）の setup-android と同じ版にそろえる。
        androidNdkVersion = "29.0.14206865";
        androidComposition = androidPkgs.androidenv.composeAndroidPackages {
          platformVersions = [ "35" ];
          buildToolsVersions = [ "35.0.0" ];
          includeNDK = true;
          ndkVersions = [ androidNdkVersion ];
        };
        # crates/zoo-crypto-ffi の ANDROID_TARGETS（frontend-compose/build.gradle.kts）と同じ。
        # rust-toolchain.toml には足さない（Android を触らない人の rustup に std を入れないため）。
        # 版は rust-toolchain.toml と同じで、sha256 も同じ版のマニフェストのもの。
        androidRustTargets = [ "aarch64-linux-android" "armv7-linux-androideabi" "x86_64-linux-android" ];
        androidToolchain = fenix.packages.${system}.combine ([ toolchain ] ++ map
          (target: (fenix.packages.${system}.targets.${target}.fromToolchainName {
            name = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;
            sha256 = "sha256-zm3dyIY2T414ZRR3EhLOvptzG6gta4WZUcawzMUWtqI=";
          }).rust-std)
          androidRustTargets);
        # ブラウザーテスト用の Chromium。CI と同じく
        # サンドボックスを外したラッパーを CHROME_BIN に渡す。Nix の chromium は Linux のみ。
        zooChrome = pkgs.writeShellScriptBin "zoo-chrome" ''
          exec ${pkgs.chromium}/bin/chromium --no-sandbox "$@"
        '';
      in
      {
        formatter = pkgs.nixpkgs-fmt;
        packages = {
          inherit zoo2-tui;
        } // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          inherit zoo2-desktop-tauri;
        };
        apps = {
          zoo2-tui = flake-utils.lib.mkApp { drv = zoo2-tui; };
        } // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          zoo2-desktop-tauri = flake-utils.lib.mkApp { drv = zoo2-desktop-tauri; };
        };
        devShells.android = pkgs.mkShell {
          name = "zoo2-android";
          nativeBuildInputs = with pkgs; [
            androidToolchain
            # nixpkgs の版（4.1.2）は CI Android の `cargo install cargo-ndk@4.1.2` と同じ。nixpkgs を上げたら合わせる。
            cargo-ndk
            gnumake
            jdk21
            androidComposition.androidsdk
          ];
          JAVA_HOME = "${pkgs.jdk21}";
          ANDROID_HOME = "${androidComposition.androidsdk}/libexec/android-sdk";
          ANDROID_NDK_HOME = "${androidComposition.androidsdk}/libexec/android-sdk/ndk/${androidNdkVersion}";
        };
        devShells.default = pkgs.stdenv.mkDerivation {
          name = "rust-env";
          nativeBuildInputs = with pkgs; [
            toolchain
            gcc
            gnumake
            nodejs_22
            pnpm
            openssl.dev
            pkg-config
            rust-analyzer
            atlas
            postgresql_16
            # Android (frontend-compose) の Gradle ビルド。Gradle 本体はラッパーを使う
            jdk21
            # infra/terraform（R2）。CI の TOFU_VERSION はこの版にそろえる
            opentofu
            sops
            age
            # Tauri 版（desktop-tauri）と E2EE の wasm（crates/zoo-crypto-wasm）。wasm-bindgen-cli の版は
            # desktop-tauri/ui/Cargo.toml と crates/zoo-crypto-wasm/Cargo.toml の wasm-bindgen（= で固定）と合わせる
            cargo-tauri
            trunk
            wasm-bindgen-cli
            # zoo-crypto のテストベクター（crates/zoo-crypto/tests/vectors/gen_secret_storage_v1.py）を、
            # zoo-crypto とは別の実装で計算し直すのに使う
            (python3.withPackages (p: [ p.cryptography ]))
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux ([ zooChrome ] ++ linuxDesktopLibs ++ tauriLibs ++ desktopScreenshotTools);
          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
          JAVA_HOME = "${pkgs.jdk21}";
          # lavapipe の Vulkan ICD。desktop-screenshot.sh が VK_ICD_FILENAMES に使う。
          ZOO_LVP_ICD = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux "${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.${pkgs.stdenv.hostPlatform.uname.processor}.json";
          CHROME_BIN = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux "${zooChrome}/bin/zoo-chrome";
          LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (pkgs.lib.makeLibraryPath (linuxDesktopLibs ++ tauriLibs));
          # WebKitGTK の TLS（glib-networking）と、GTK のファイル選択などが使う schema。
          GIO_MODULE_DIR = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux "${pkgs.glib-networking}/lib/gio/modules";
          shellHook = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
            export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
          '';
        };
      }
    );
}
