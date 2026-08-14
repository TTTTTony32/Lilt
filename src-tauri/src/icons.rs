#[cfg(windows)]
use std::io::Cursor;

use tauri::image::Image;

#[cfg(windows)]
use crate::diagnostics;

const HIGH_RES_ICON_PNG: &[u8] = include_bytes!("../icons/128x128@2x.png");
#[cfg(not(windows))]
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");
#[cfg(windows)]
const TRAY_ICON_ICO: &[u8] = include_bytes!("../icons/icon.ico");

pub(crate) fn high_resolution_icon() -> tauri::Result<Image<'static>> {
    decode_png(HIGH_RES_ICON_PNG, "高分辨率图标")
}

pub(crate) fn tray_icon() -> tauri::Result<Image<'static>> {
    #[cfg(windows)]
    {
        decode_windows_tray_icon()
    }

    #[cfg(not(windows))]
    {
        decode_png(TRAY_ICON_PNG, "托盘图标")
    }
}

#[cfg(windows)]
fn decode_windows_tray_icon() -> tauri::Result<Image<'static>> {
    let (dpi, target_size) = startup_tray_target_size()?;
    let icon_dir = ico::IconDir::read(Cursor::new(TRAY_ICON_ICO))
        .map_err(|error| icon_error(format!("读取托盘 ICO 失败：{error}")))?;
    let mut decoded_entries = Vec::new();
    for entry in icon_dir.entries() {
        match entry.decode() {
            Ok(image) => decoded_entries.push((entry.width(), entry.height(), image)),
            Err(error) => diagnostics::warn(format!(
                "tray.icon.layer_decode_failed width={} height={} reason={error}",
                entry.width(),
                entry.height()
            )),
        }
    }
    let sizes = decoded_entries
        .iter()
        .map(|(width, height, _)| (*width, *height))
        .collect::<Vec<_>>();
    let selected_index = select_tray_icon_index(&sizes, target_size)
        .ok_or_else(|| icon_error("托盘 ICO 没有可用的正方形图层"))?;
    let (width, height, image) = decoded_entries.swap_remove(selected_index);
    if width < target_size {
        diagnostics::warn(format!(
            "tray.icon.downgraded dpi={} target_size={} selected_size={} reason=max_embedded_layer",
            dpi, target_size, width
        ));
    } else {
        diagnostics::info(format!(
            "tray.icon.selected dpi={} target_size={} selected_size={} source=icon.ico",
            dpi, target_size, width
        ));
    }
    Ok(Image::new_owned(image.into_rgba_data(), width, height))
}

#[cfg(windows)]
fn startup_tray_target_size() -> tauri::Result<(u32, u32)> {
    use windows::Win32::UI::{
        HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi},
        WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON},
    };

    let dpi = unsafe { GetDpiForSystem() };
    let target_size = if dpi == 0 {
        unsafe { GetSystemMetrics(SM_CXSMICON) }
    } else {
        unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }
    };
    if target_size <= 0 {
        return Err(icon_error(format!(
            "获取托盘图标目标尺寸失败：dpi={dpi} target_size={target_size}"
        )));
    }
    Ok((dpi, target_size as u32))
}

fn select_tray_icon_index(sizes: &[(u32, u32)], target_size: u32) -> Option<usize> {
    let mut selected: Option<usize> = None;
    let mut largest: Option<usize> = None;
    for (index, &(width, height)) in sizes.iter().enumerate() {
        if width == 0 || width != height {
            continue;
        }
        if largest.is_none_or(|largest_index| sizes[largest_index].0 < width) {
            largest = Some(index);
        }
        if width >= target_size
            && selected.is_none_or(|selected_index| sizes[selected_index].0 > width)
        {
            selected = Some(index);
        }
    }
    selected.or(largest)
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

#[cfg(test)]
mod tests {
    use super::select_tray_icon_index;

    #[test]
    fn selects_exact_size() {
        assert_eq!(
            select_tray_icon_index(&[(16, 16), (24, 24), (32, 32)], 24),
            Some(1)
        );
    }

    #[test]
    fn selects_smallest_size_above_target() {
        assert_eq!(
            select_tray_icon_index(&[(16, 16), (24, 24), (48, 48)], 20),
            Some(1)
        );
        assert_eq!(
            select_tray_icon_index(&[(16, 16), (24, 24), (48, 48)], 40),
            Some(2)
        );
    }

    #[test]
    fn falls_back_to_largest_size_when_target_is_too_large() {
        assert_eq!(
            select_tray_icon_index(&[(16, 16), (64, 64), (32, 32)], 128),
            Some(1)
        );
    }

    #[test]
    fn ignores_non_square_and_empty_sizes() {
        assert_eq!(
            select_tray_icon_index(&[(0, 0), (32, 24), (24, 24)], 16),
            Some(2)
        );
        assert_eq!(select_tray_icon_index(&[], 16), None);
    }
}
