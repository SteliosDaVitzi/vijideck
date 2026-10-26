use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
const NAME: &str = "vijideck";
const TITLE: &str = "VijiDeck";

fn run(command: &mut Command) -> Result<String> {
    eprintln!("Running: {command:?}");
    let output = command.stderr(Stdio::inherit()).output()?;
    if !output.status.success() {
        return Err(format!(
            "Command failed ({}): {command:?}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result {
    fs::create_dir_all(path.parent().ok_or("Path has no parent")?)?;
    fs::write(path, contents)?;
    Ok(())
}

fn version() -> Result<&'static str> {
    let manifest = include_str!("../../Cargo.toml");
    let package = manifest
        .split("[package]")
        .nth(1)
        .ok_or("Missing package")?
        .split("\n[")
        .next()
        .ok_or("Missing package section")?;
    package
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "version").then(|| value.trim().trim_matches('"'))
        })
        .ok_or_else(|| "Missing package version".into())
}

fn windows_compiler() -> Result<PathBuf> {
    let compiler = env::var_os("ISCC")
        .map(PathBuf::from)
        .or_else(|| {
            ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]
                .iter()
                .filter_map(env::var_os)
                .map(PathBuf::from)
                .flat_map(|dir| {
                    [
                        dir.join("Inno Setup 6/ISCC.exe"),
                        dir.join("Programs/Inno Setup 6/ISCC.exe"),
                    ]
                })
                .find(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("ISCC.exe"));
    // ISCC returns 1 for /? even when it successfully displays help.
    let help = Command::new(&compiler).arg("/?").output().map_err(|e| {
        format!("Cannot run Inno Setup compiler at {}. Install Inno Setup 6 or set ISCC to ISCC.exe. {e}", compiler.display())
    })?;
    let help_displayed = help.status.code() == Some(1)
        && String::from_utf8_lossy(&help.stdout).contains("Inno Setup");
    if !(help.status.success() || help_displayed) {
        return Err(format!(
            "Inno Setup compiler check failed at {} ({}): {}{}",
            compiler.display(),
            help.status,
            String::from_utf8_lossy(&help.stdout),
            String::from_utf8_lossy(&help.stderr)
        )
        .into());
    }
    Ok(compiler)
}

fn windows(binary: &Path, out: &Path, version: &str, arch: &str, root: &Path) -> Result {
    let compiler = windows_compiler()?;
    let icon = root.join("packaging/vijideck.ico");
    run(Command::new(compiler)
        .arg(format!("/DAppVersion={version}"))
        .arg(format!("/DBinaryPath={}", binary.display()))
        .arg(format!("/DIconPath={}", icon.display()))
        .arg(format!("/DOutputPath={}", out.display()))
        .arg(root.join("packaging/windows.iss")))?;
    run(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
        "$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:VIJIDECK_BINARY -DestinationPath $env:VIJIDECK_ARCHIVE -Force"])
        .env("VIJIDECK_BINARY", binary).env("VIJIDECK_ARCHIVE", out.join(format!("{NAME}-{version}-windows-{arch}.zip"))))?;
    Ok(())
}

fn macos(binary: &Path, out: &Path, version: &str, arch: &str, root: &Path, stage: &Path) -> Result {
    let image = stage.join("image");
    let app = image.join(format!("{TITLE}.app"));
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    let resources = contents.join("Resources");
    let iconset = stage.join("VijiDeck.iconset");
    fs::create_dir_all(&iconset)?;
    for (name, pixels) in [
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
    ] {
        run(Command::new("sips")
            .args(["-z", &pixels.to_string(), &pixels.to_string()])
            .arg(root.join("Glossy Donut Chart in Blue Tray.png"))
            .args(["--out"])
            .arg(iconset.join(name)))?;
    }
    fs::create_dir_all(&resources)?;
    run(Command::new("iconutil")
        .args(["-c", "icns"])
        .arg(&iconset)
        .arg("-o")
        .arg(resources.join("AppIcon.icns")))?;
    fs::copy(binary, contents.join("MacOS").join(NAME))?;
    write(
        &contents.join("Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>{TITLE}</string>
<key>CFBundleDisplayName</key><string>{TITLE}</string>
<key>CFBundleIdentifier</key><string>app.vijideck.desktop</string>
<key>CFBundleExecutable</key><string>{NAME}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#
        ),
    )?;
    run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&app))?;
    run(Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(&app))?;
    run(Command::new("ln")
        .args(["-s", "/Applications"])
        .arg(image.join("Applications")))?;
    run(Command::new("hdiutil")
        .args(["create", "-volname", TITLE, "-srcfolder"])
        .arg(image)
        .args(["-format", "UDZO", "-ov"])
        .arg(out.join(format!("{NAME}-{version}-macos-{arch}.dmg"))))?;
    Ok(())
}

fn linux(binary: &Path, out: &Path, version: &str, arch: &str, root: &Path, stage: &Path) -> Result {
    let package = stage.join("package");
    let installed = package.join("usr/bin").join(NAME);
    fs::create_dir_all(installed.parent().ok_or("Missing parent")?)?;
    fs::copy(binary, &installed)?;
    run(Command::new("chmod").arg("755").arg(&installed))?;
    let icon = package.join(format!("usr/share/icons/hicolor/1024x1024/apps/{NAME}.png"));
    fs::create_dir_all(icon.parent().ok_or("Missing icon parent")?)?;
    fs::copy(root.join("Glossy Donut Chart in Blue Tray.png"), &icon)?;
    let desktop = package.join(format!("usr/share/applications/{NAME}.desktop"));
    write(
        &desktop,
        format!(
            "[Desktop Entry]\nType=Application\nName={TITLE}\nComment=System overview, ports and disk cleanup\nExec={NAME}\nIcon={NAME}\nTerminal=false\nCategories=System;Utility;\n"
        ),
    )?;
    run(Command::new("desktop-file-validate").arg(&desktop))?;
    write(
        &stage.join("debian/control"),
        format!(
            "Source: {NAME}\nMaintainer: VijiDeck contributors <noreply@example.invalid>\n\nPackage: {NAME}\nArchitecture: any\nDescription: VijiDeck desktop utilities\n"
        ),
    )?;
    let deps = run(Command::new("dpkg-shlibdeps")
        .arg("-O")
        .arg(format!("-e{}", binary.display()))
        .current_dir(stage))?;
    let deps = deps
        .lines()
        .find_map(|line| line.strip_prefix("shlibs:Depends="))
        .ok_or("Missing Debian dependencies")?;
    let deb_arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return Err("Unsupported architecture".into()),
    };
    write(
        &package.join("DEBIAN/control"),
        format!(
            "Package: {NAME}\nVersion: {version}\nArchitecture: {deb_arch}\nMaintainer: VijiDeck contributors <noreply@example.invalid>\nSection: utils\nPriority: optional\nDepends: {deps}, libx11-6, libxcursor1, libxi6, libxrandr2, libgl1, libwayland-client0, libxkbcommon0\nDescription: {TITLE}\n Native system overview, port management and disk cleanup.\n"
        ),
    )?;
    run(Command::new("dpkg-deb")
        .args(["--root-owner-group", "--build"])
        .arg(&package)
        .arg(out.join(format!("{NAME}-{version}-linux-{arch}.deb"))))?;
    let portable = stage.join(format!("{NAME}-{version}"));
    fs::create_dir(&portable)?;
    fs::copy(binary, portable.join(NAME))?;
    fs::copy(desktop, portable.join(format!("{NAME}.desktop")))?;
    let portable_icon = portable.join(format!("share/icons/hicolor/1024x1024/apps/{NAME}.png"));
    fs::create_dir_all(portable_icon.parent().ok_or("Missing portable icon parent")?)?;
    fs::copy(root.join("Glossy Donut Chart in Blue Tray.png"), portable_icon)?;
    run(Command::new("tar")
        .arg("-czf")
        .arg(out.join(format!("{NAME}-{version}-linux-{arch}.tar.gz")))
        .arg("-C")
        .arg(stage)
        .arg(format!("{NAME}-{version}")))?;
    Ok(())
}

fn main() -> Result {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "cargo xtask package [--skip-build]\nBuild native installers and portable archives using Rust and native OS tools."
        );
        return Ok(());
    }
    if args.first().map(String::as_str) != Some("package")
        || args.len() > 2
        || args.get(1).is_some_and(|arg| arg != "--skip-build")
    {
        return Err("Usage: cargo xtask package [--skip-build]".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Missing project root")?;
    let version = version()?;
    if let Ok(tag) = env::var("GITHUB_REF")
        && tag.starts_with("refs/tags/")
        && tag != format!("refs/tags/v{version}")
    {
        return Err(format!("Release tag must match Cargo.toml: v{version}").into());
    }
    let rustc = run(Command::new("rustc").arg("-vV"))?;
    let host = rustc
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("Missing Rust host")?;
    let arch = host.split('-').next().ok_or("Missing architecture")?;
    if !["x86_64", "aarch64"].contains(&arch) {
        return Err(format!("Unsupported host: {host}").into());
    }
    let os = env::consts::OS;
    if !["windows", "macos", "linux"].contains(&os) {
        return Err(format!("Unsupported OS: {os}").into());
    }
    if os == "windows" {
        if arch != "x86_64" {
            return Err("Windows packages currently require x86_64".into());
        }
        windows_compiler()?;
    }
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let target = if target.is_absolute() {
        target
    } else {
        root.join(target)
    };
    if !args.iter().any(|arg| arg == "--skip-build") {
        let status = Command::new("cargo")
            .args(["build", "--locked", "--release", "--target", host])
            .arg("--target-dir")
            .arg(&target)
            .current_dir(root)
            .status()?;
        if !status.success() {
            return Err("Application build failed".into());
        }
    }
    let binary = target
        .join(host)
        .join("release")
        .join(format!("{NAME}{}", env::consts::EXE_SUFFIX));
    if !binary.is_file() {
        return Err(format!("Missing release executable: {}", binary.display()).into());
    }
    let out = root.join("dist").join(format!("{version}-{os}-{arch}"));
    fs::create_dir_all(&out)?;
    let stage = out.join(format!(".stage-{}", std::process::id()));
    fs::create_dir(&stage)?;
    let result = match os {
        "windows" => windows(&binary, &out, version, arch, root),
        "macos" => macos(&binary, &out, version, arch, root, &stage),
        "linux" => linux(&binary, &out, version, arch, root, &stage),
        _ => unreachable!(),
    };
    fs::remove_dir_all(&stage)?;
    result?;
    let mut files = fs::read_dir(&out)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    let mut checksums = String::new();
    for path in files.into_iter().filter(|path| {
        path.is_file()
            && path
                .file_name()
                .is_some_and(|name| name != "SHA256SUMS.txt")
    }) {
        let output = if os == "windows" {
            run(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; (Get-FileHash -LiteralPath $env:VIJIDECK_PACKAGE -Algorithm SHA256).Hash"]).env("VIJIDECK_PACKAGE", &path))?
        } else if os == "macos" {
            run(Command::new("shasum").args(["-a", "256"]).arg(&path))?
        } else {
            run(Command::new("sha256sum").arg(&path))?
        };
        let hash = output
            .split_whitespace()
            .next()
            .ok_or("Missing checksum")?
            .to_lowercase();
        checksums.push_str(&format!(
            "{hash}  {}\n",
            path.file_name()
                .ok_or("Missing filename")?
                .to_string_lossy()
        ));
    }
    write(&out.join("SHA256SUMS.txt"), checksums)?;
    println!("Packages: {}", out.display());
    Ok(())
}
