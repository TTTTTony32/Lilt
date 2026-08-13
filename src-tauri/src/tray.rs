#![cfg(desktop)]

use tauri::{menu::MenuBuilder, tray::TrayIconBuilder, AppHandle, Manager};

pub const TRAY_ID: &str = "main-tray";
pub const MENU_ID_OPEN: &str = "tray-open";
pub const MENU_ID_EXIT: &str = "tray-exit";

pub fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    let handle = app.clone();
    let menu = MenuBuilder::new(&handle)
        .text(MENU_ID_OPEN, "打开 Lilt")
        .separator()
        .text(MENU_ID_EXIT, "退出")
        .build()?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Lilt");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let _tray = builder.build(&handle)?;
    Ok(())
}

pub fn register_menu_handler(app: &AppHandle) {
    app.on_menu_event(|app_handle, event| match event.id().as_ref() {
        MENU_ID_OPEN => {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
        MENU_ID_EXIT => app_handle.exit(0),
        _ => {}
    });
}
