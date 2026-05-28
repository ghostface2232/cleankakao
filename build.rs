fn main() {
    println!("cargo:rerun-if-changed=assets/icon_active.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");

    if !cfg!(target_os = "windows") {
        return;
    }

    let package_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let windows_version = windows_version_number(&package_version);

    let mut resource = winres::WindowsResource::new();
    resource
        .set_icon("assets/icon_active.ico")
        .set("FileDescription", "CleanKakao")
        .set("ProductName", "CleanKakao")
        .set("LegalCopyright", "MIT License")
        .set("FileVersion", &package_version)
        .set("ProductVersion", &package_version)
        .set_version_info(winres::VersionInfo::FILEVERSION, windows_version)
        .set_version_info(winres::VersionInfo::PRODUCTVERSION, windows_version);

    resource
        .compile()
        .expect("failed to compile Windows resources");
}

fn windows_version_number(version: &str) -> u64 {
    let core = version
        .split(|ch| ch == '-' || ch == '+')
        .next()
        .unwrap_or(version);
    let mut parts = core
        .split('.')
        .map(|part| part.parse::<u16>().unwrap_or(0) as u64);

    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    let build = parts.next().unwrap_or(0);

    (major << 48) | (minor << 32) | (patch << 16) | build
}
