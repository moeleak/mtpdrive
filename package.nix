{
  lib,
  stdenv,
  rustPlatform,
  libiconv,
  writableTmpDirAsHomeHook,
}:
let
  src = lib.cleanSource ./.;
  releaseDir = "target/${stdenv.hostPlatform.rust.rustcTarget}/release";
in
rec {
  default = mtpdrive;

  mtpdrive = rustPlatform.buildRustPackage {
    pname = "mtpdrive";
    version = "0.1.0";
    inherit src;

    cargoLock.lockFile = ./Cargo.lock;

    cargoBuildFlags = [
      "--workspace"
      "--exclude"
      "xtask"
    ];

    cargoTestFlags = [
      "--workspace"
      "--exclude"
      "xtask"
    ];

    nativeBuildInputs = [ writableTmpDirAsHomeHook ];
    buildInputs = lib.optionals stdenv.isDarwin [ libiconv ];

    env = lib.optionalAttrs stdenv.isDarwin {
      MACOSX_DEPLOYMENT_TARGET = "13.0";
    };

    installPhase = ''
      runHook preInstall

      install -Dm755 ${releaseDir}/mtpdrive "$out/bin/mtpdrive"
      if [ -x ${releaseDir}/mtpdrive-app ]; then
        install -Dm755 ${releaseDir}/mtpdrive-app "$out/bin/mtpdrive-app"
      fi

      runHook postInstall
    '';

    meta = {
      description = "Expose Android MTP devices as a local macOS NFS volume";
      homepage = "https://github.com/moeleak/mtpdrive";
      license = lib.licenses.mit;
      mainProgram = "mtpdrive";
      platforms = lib.platforms.darwin;
    };
  };
}
