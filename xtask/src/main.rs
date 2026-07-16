use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use image::imageops::FilterType;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const APP_NAME: &str = "MTPDrive";
const GUI_BINARY: &str = "mtpdrive-app";
const CLI_BINARY: &str = "mtpdrive";
const TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

#[derive(Debug, Parser)]
#[command(about = "MTPDrive macOS packaging tasks")]
struct Xtask {
    #[command(subcommand)]
    command: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Build a signed Universal macOS application bundle.
    App {
        /// Reuse prebuilt binaries under target/<triple>/release.
        #[arg(long)]
        skip_build: bool,
    },
    /// Build a signed Universal app and compressed DMG.
    Dmg {
        /// Reuse prebuilt binaries under target/<triple>/release.
        #[arg(long)]
        skip_build: bool,
    },
    /// Regenerate AppIcon.icns from the transparent source PNG.
    Icon,
}

fn main() -> Result<()> {
    let task = Xtask::parse().command;
    ensure_macos()?;
    let root = workspace_root()?;
    match task {
        Task::App { skip_build } => {
            let app = build_app(&root, skip_build)?;
            println!("{}", app.display());
        }
        Task::Dmg { skip_build } => {
            let app = build_app(&root, skip_build)?;
            let dmg = build_dmg(&root, &app)?;
            println!("{}", dmg.display());
        }
        Task::Icon => {
            let icon = build_icon(&root)?;
            println!("{}", icon.display());
        }
    }
    Ok(())
}

fn ensure_macos() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        bail!("macOS application and DMG packaging must run on macOS")
    }
}

fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("xtask is not inside the workspace")
}

fn build_app(root: &Path, skip_build: bool) -> Result<PathBuf> {
    if !skip_build {
        for target in TARGETS {
            build_target(root, target)?;
        }
    }
    for target in TARGETS {
        require_binary(root, target, GUI_BINARY)?;
        require_binary(root, target, CLI_BINARY)?;
    }

    let dist = root.join("dist");
    let build = dist.join("build");
    let app = dist.join(format!("{APP_NAME}.app"));
    if build.exists() {
        fs::remove_dir_all(&build)?;
    }
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    let macos = app.join("Contents/MacOS");
    let helpers = app.join("Contents/Helpers");
    let resources = app.join("Contents/Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&helpers)?;
    fs::create_dir_all(&resources)?;
    fs::create_dir_all(&build)?;

    let universal_gui = macos.join(APP_NAME);
    let universal_cli = helpers.join(CLI_BINARY);
    lipo(root, GUI_BINARY, &universal_gui)?;
    lipo(root, CLI_BINARY, &universal_cli)?;
    verify_portable_macho(&universal_gui)?;
    verify_portable_macho(&universal_cli)?;
    make_executable(&universal_gui)?;
    make_executable(&universal_cli)?;

    let info_template = fs::read_to_string(root.join("crates/mtpdrive-app/Info.plist.in"))?;
    let info = info_template.replace("@VERSION@", env!("CARGO_PKG_VERSION"));
    fs::write(app.join("Contents/Info.plist"), info)?;
    fs::write(app.join("Contents/PkgInfo"), "APPLMTPD")?;

    let icon = build_icon(root)?;
    fs::copy(icon, resources.join("AppIcon.icns"))?;
    fs::copy(root.join("LICENSE"), resources.join("LICENSE.txt"))?;
    fs::copy(
        root.join("THIRD_PARTY_NOTICES.md"),
        resources.join("THIRD_PARTY_NOTICES.md"),
    )?;

    ad_hoc_sign(&universal_cli)?;
    ad_hoc_sign(&universal_gui)?;
    run(
        Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(&app),
        "sign app bundle",
    )?;
    run(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict", "--verbose=2"])
            .arg(&app),
        "verify app signature",
    )?;
    run(
        Command::new("/usr/bin/lipo")
            .arg("-info")
            .arg(&universal_gui),
        "verify Universal GUI binary",
    )?;
    Ok(app)
}

fn build_target(root: &Path, target: &str) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let linker = root.join("scripts/macos-linker.sh");
    if !linker.is_file() {
        bail!("missing macOS linker wrapper {}", linker.display());
    }
    run(
        Command::new(&cargo).current_dir(root).args([
            "clean",
            "--release",
            "--target",
            target,
            "-p",
            "mtpdrive-app",
            "-p",
            "mtpdrive-cli",
        ]),
        &format!("clean {target} application artifacts"),
    )?;

    let linker_variable = match target {
        "aarch64-apple-darwin" => "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER",
        "x86_64-apple-darwin" => "CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER",
        _ => bail!("unsupported packaging target {target}"),
    };
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .env("MACOSX_DEPLOYMENT_TARGET", "13.0")
        .env(linker_variable, linker)
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            target,
            "-p",
            "mtpdrive-app",
            "-p",
            "mtpdrive-cli",
        ]);
    // Nix development shells inject store library search paths. They are correct for
    // Nix packages, but a standalone DMG must resolve Apple SDK libraries from macOS.
    for variable in [
        "NIX_CFLAGS_COMPILE",
        "NIX_CFLAGS_COMPILE_FOR_BUILD",
        "NIX_LDFLAGS",
        "NIX_LDFLAGS_FOR_BUILD",
    ] {
        command.env_remove(variable);
    }
    run(&mut command, &format!("build {target}"))
}

fn require_binary(root: &Path, target: &str, binary: &str) -> Result<PathBuf> {
    let path = root
        .join("target")
        .join(target)
        .join("release")
        .join(binary);
    if !path.is_file() {
        bail!("missing {}; build it or omit --skip-build", path.display());
    }
    Ok(path)
}

fn lipo(root: &Path, binary: &str, output: &Path) -> Result<()> {
    let arm = require_binary(root, TARGETS[0], binary)?;
    let intel = require_binary(root, TARGETS[1], binary)?;
    run(
        Command::new("/usr/bin/lipo")
            .arg("-create")
            .arg(&arm)
            .arg(&intel)
            .arg("-output")
            .arg(output),
        &format!("create Universal {binary}"),
    )
}

fn verify_portable_macho(path: &Path) -> Result<()> {
    let output = Command::new("/usr/bin/otool")
        .arg("-L")
        .arg(path)
        .output()
        .with_context(|| format!("inspect dynamic dependencies for {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "otool failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let dependencies = String::from_utf8_lossy(&output.stdout);
    if dependencies.contains("/nix/store/") {
        bail!(
            "{} contains a non-portable Nix store dependency:\n{}",
            path.display(),
            dependencies
        );
    }
    Ok(())
}

fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn ad_hoc_sign(path: &Path) -> Result<()> {
    run(
        Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(path),
        &format!("sign {}", path.display()),
    )
}

fn build_icon(root: &Path) -> Result<PathBuf> {
    let source = root.join("crates/mtpdrive-app/assets/icon.png");
    let iconset = root.join("dist/build/AppIcon.iconset");
    let output = root.join("dist/build/AppIcon.icns");
    if iconset.exists() {
        fs::remove_dir_all(&iconset)?;
    }
    fs::create_dir_all(&iconset)?;
    let image = image::open(&source)
        .with_context(|| format!("open icon source {}", source.display()))?
        .into_rgba8();
    let definitions = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ];
    for (name, size) in definitions {
        image::imageops::resize(&image, size, size, FilterType::Lanczos3)
            .save(iconset.join(name))?;
    }
    if output.exists() {
        fs::remove_file(&output)?;
    }
    run(
        Command::new("/usr/bin/iconutil")
            .args([OsStr::new("-c"), OsStr::new("icns")])
            .arg(&iconset)
            .arg("-o")
            .arg(&output),
        "create AppIcon.icns",
    )?;
    Ok(output)
}

fn build_dmg(root: &Path, app: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::symlink;

    let dist = root.join("dist");
    let staging = dist.join("dmg-root");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    run(
        Command::new("/usr/bin/ditto")
            .arg(app)
            .arg(staging.join(format!("{APP_NAME}.app"))),
        "stage app for DMG",
    )?;
    symlink("/Applications", staging.join("Applications"))?;

    let dmg = dist.join(format!(
        "{APP_NAME}-{}-universal.dmg",
        env!("CARGO_PKG_VERSION")
    ));
    if dmg.exists() {
        fs::remove_file(&dmg)?;
    }
    run(
        Command::new("/usr/bin/hdiutil")
            .args(["create", "-ov", "-format", "UDZO", "-volname", APP_NAME])
            .arg("-srcfolder")
            .arg(&staging)
            .arg(&dmg),
        "create compressed DMG",
    )?;
    write_sha256(&dmg)?;
    Ok(dmg)
}

fn write_sha256(path: &Path) -> Result<()> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = format!("{:x}", hasher.finalize());
    let filename = path
        .file_name()
        .and_then(OsStr::to_str)
        .context("DMG path has no UTF-8 filename")?;
    fs::write(
        path.with_extension("dmg.sha256"),
        format!("{digest}  {filename}\n"),
    )?;
    Ok(())
}

fn run(command: &mut Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("could not {description}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("failed to {description}: {status}")
    }
}
