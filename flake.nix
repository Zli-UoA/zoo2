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
        zoo2-tui = rustPlatform.buildRustPackage {
          pname = "zoo2-tui";
          version = (builtins.fromTOML (builtins.readFile ./tui/Cargo.toml)).package.version;
          src = ./tui;
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
        # Compose Web (Kotlin/Wasm) の単体テストは Karma で Chrome を使う。CI と同じく
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
            # Compose Web (frontend-compose) の Gradle ビルド。Gradle 本体はラッパーを使う
            jdk21
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ zooChrome ];
          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
          JAVA_HOME = "${pkgs.jdk21}";
          CHROME_BIN = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux "${zooChrome}/bin/zoo-chrome";
        };
      }
    );
}
