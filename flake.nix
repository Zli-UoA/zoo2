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
        # Compose Web (Kotlin/Wasm) の単体テストは Karma で Chrome を使う。CI と同じく
        # サンドボックスを外したラッパーを CHROME_BIN に渡す。Nix の chromium は Linux のみ。
        zooChrome = pkgs.writeShellScriptBin "zoo-chrome" ''
          exec ${pkgs.chromium}/bin/chromium --no-sandbox "$@"
        '';
      in
      {
        formatter = pkgs.nixpkgs-fmt;
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
