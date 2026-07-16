{
  description = "MTPDrive - open Android MTP devices in Finder through local NFSv3";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        inherit (pkgs) lib;
        nativePackages = pkgs.callPackage ./package.nix { };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "clippy"
            "rust-analyzer"
            "rust-src"
            "rustfmt"
          ];
          targets = [
            "aarch64-apple-darwin"
            "x86_64-apple-darwin"
          ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              rust
              cargo-audit
              cargo-deny
              git
              gh
              jq
              pkg-config
            ]
            ++ lib.optionals stdenv.isDarwin [
              libiconv
            ];

          shellHook = ''
            export MACOSX_DEPLOYMENT_TARGET=13.0
          '';
        };

        packages = {
          default = nativePackages.mtpdrive;
          inherit (nativePackages) mtpdrive;
        };

        checks = {
          inherit (nativePackages) mtpdrive;
        };

        formatter = pkgs.nixfmt;
      }
    );
}
