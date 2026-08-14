use tauri::image::Image;

const HIGH_RES_ICON_PNG: &[u8] = include_bytes!("../icons/128x128@2x.png");
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");

pub(crate) fn high_resolution_icon() -> tauri::Result<Image<'static>> {
    decode_png(HIGH_RES_ICON_PNG, "高分辨率图标")
}

pub(crate) fn tray_icon() -> tauri::Result<Image<'static>> {
    decode_png(TRAY_ICON_PNG, "托盘图标")
}

fn decode_png(bytes: &[u8], name: &str) -> tauri::Result<Image<'static>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| icon_error(format!("读取{name}失败：{error}")))?;
    let mut rgba = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| icon_error(format!("{name}尺寸超出可处理范围")))?
    ];
    let output = reader
        .next_frame(&mut rgba)
        .map_err(|error| icon_error(format!("解码{name}失败：{error}")))?;
    if output.color_type != png::ColorType::Rgba {
        return Err(icon_error(format!("{name}不是 RGBA 格式")));
    }
    rgba.truncate(output.buffer_size());
    Ok(Image::new_owned(rgba, output.width, output.height))
}

fn icon_error(message: impl Into<String>) -> tauri::Error {
    tauri::Error::Io(std::io::Error::other(message.into()))
}
