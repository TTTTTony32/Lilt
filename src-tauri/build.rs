const DEFAULT_ICON: &[u8] = &[
    0, 0, 1, 0, 1, 0, 1, 1, 0, 0, 1, 0, 32, 0, 48, 0, 0, 0, 22, 0, 0, 0, 40, 0, 0, 0, 1, 0, 0, 0,
    2, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 120, 215, 255, 0, 0, 0, 0,
];

fn ensure_default_icon() {
    let icons_dir = std::path::Path::new("icons");
    let icon_path = icons_dir.join("icon.ico");
    if icon_path.exists() {
        return;
    }

    std::fs::create_dir_all(icons_dir).expect("failed to create the Tauri icons directory");
    std::fs::write(icon_path, DEFAULT_ICON).expect("failed to create the default Tauri icon");
}

fn main() {
    ensure_default_icon();
    tauri_build::build()
}
