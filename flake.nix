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
        # ブラウザーテスト用の Chromium。CI と同じく
        # サンドボックスを外したラッパーを CHROME_BIN に渡す。Nix の chromium は Linux のみ。
        zooChrome = pkgs.writeShellScriptBin "zoo-chrome" ''
          exec ${pkgs.chromium}/bin/chromium --no-sandbox "$@"
        '';
      in
      {
        formatter = pkgs.nixpkgs-fmt;
        packages.zoo2-tui = zoo2-tui;
        apps.zoo2-tui = flake-utils.lib.mkApp { drv = zoo2-tui; };
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
