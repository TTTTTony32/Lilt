use tauri::image::Image;

const HIGH_RES_ICON_PNG: &[u8] = include_bytes!("../icons/128x128@2x.png");

pub(crate) fn high_resolution_icon() -> tauri::Result<Image<'static>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(HIGH_RES_ICON_PNG));
    let mut reader = decoder
        .read_info()
        .map_err(|error| icon_error(format!("读取高分辨率图标失败：{error}")))?;
    let mut rgba = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| icon_error("高分辨率图标尺寸超出可处理范围"))?
    ];
    let output = reader
        .next_frame(&mut rgba)
        .map_err(|error| icon_error(format!("解码高分辨率图标失败：{error}")))?;
    if output.color_type != png::ColorType::Rgba {
        return Err(icon_error("高分辨率图标不是 RGBA 格式"));
    }
    rgba.truncate(output.buffer_size());
    Ok(Image::new_owned(rgba, output.width, output.height))
}

fn icon_error(message: impl Into<String>) -> tauri::Error {
    tauri::Error::Io(std::io::Error::other(message.into()))
}
