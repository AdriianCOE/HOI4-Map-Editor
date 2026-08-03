#[cfg(windows)]
fn main() {
    const ICON_PATH: &str = "assets/app-icon.ico";

    println!("cargo:rerun-if-changed={ICON_PATH}");
    if !std::path::Path::new(ICON_PATH).is_file() {
        panic!("missing Windows application icon: {ICON_PATH}");
    }

    let mut resource = winres::WindowsResource::new();
    resource.set_icon(ICON_PATH);
    resource
        .compile()
        .expect("failed to embed Windows application icon");
}

#[cfg(not(windows))]
fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
}
