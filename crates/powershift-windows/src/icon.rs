use crate::{PowerError, PowerResult};
use base64::Engine;

pub fn png_data_url_from_executable(path: &str) -> PowerResult<String> {
    let png = executable_icon_png(path)?;
    Ok(png_data_url(&png))
}

pub fn png_data_url(bytes: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[cfg(windows)]
fn executable_icon_png(path: &str) -> PowerResult<Vec<u8>> {
    use image::{ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;
    use std::mem::{size_of, zeroed};
    use windows::Win32::Graphics::Gdi::{
        GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let icon = IconHandle(extract_best_icon(path)?);
    let mut info = ICONINFO::default();
    unsafe {
        GetIconInfo(icon.0, &mut info).map_err(|error| PowerError::Parse(error.to_string()))?
    };
    let color_bitmap = BitmapHandle(info.hbmColor);
    let _mask_bitmap = BitmapHandle(info.hbmMask);

    if color_bitmap.0 .0.is_null() {
        return Err(PowerError::Parse(
            "El icono no tiene bitmap de color".to_string(),
        ));
    }

    let mut bitmap: BITMAP = unsafe { zeroed() };
    let object_size = unsafe {
        GetObjectW(
            HGDIOBJ(color_bitmap.0 .0),
            size_of::<BITMAP>() as i32,
            Some((&mut bitmap as *mut BITMAP).cast()),
        )
    };
    if object_size == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight <= 0 {
        return Err(PowerError::Parse(
            "No se pudo leer el bitmap del icono".to_string(),
        ));
    }

    let width = bitmap.bmWidth as u32;
    let height = bitmap.bmHeight as u32;
    let mut info_header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    let hdc = unsafe { GetDC(None) };
    let dib_lines = unsafe {
        GetDIBits(
            hdc,
            color_bitmap.0,
            0,
            height,
            Some(bgra.as_mut_ptr().cast()),
            &mut info_header,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        ReleaseDC(None, hdc);
    }
    if dib_lines == 0 {
        return Err(PowerError::Parse(
            "No se pudo convertir el icono a pixeles".to_string(),
        ));
    }

    let mut rgba = Vec::with_capacity(bgra.len());
    for pixel in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }

    let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, rgba)
        .ok_or_else(|| PowerError::Parse("El buffer del icono no es valido".to_string()))?;
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| PowerError::Parse(error.to_string()))?;
    Ok(png.into_inner())
}

#[cfg(windows)]
fn extract_best_icon(path: &str) -> PowerResult<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ExtractIconExW;
    use windows::Win32::UI::WindowsAndMessaging::HICON;

    if let Some(icon) = extract_private_icon(path) {
        return Ok(icon);
    }

    let wide_path = wide_null(path);
    let mut large_icon = HICON::default();
    let extracted = unsafe {
        ExtractIconExW(
            PCWSTR(wide_path.as_ptr()),
            0,
            Some(&mut large_icon),
            None,
            1,
        )
    };

    if extracted == 0 || large_icon.0.is_null() {
        return Err(PowerError::Parse(format!(
            "No se pudo extraer icono de {path}"
        )));
    }

    Ok(large_icon)
}

#[cfg(windows)]
fn extract_private_icon(path: &str) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::UI::WindowsAndMessaging::{PrivateExtractIconsW, HICON};

    let wide_path = wide_null(path);
    if wide_path.len() > 260 {
        return None;
    }

    let mut path_buffer = [0u16; 260];
    path_buffer[..wide_path.len()].copy_from_slice(&wide_path);
    let mut icons = [HICON::default()];
    let mut icon_id = 0u32;
    let extracted = unsafe {
        PrivateExtractIconsW(
            &path_buffer,
            0,
            256,
            256,
            Some(&mut icons),
            Some(&mut icon_id),
            0,
        )
    };

    if extracted > 0 && !icons[0].0.is_null() {
        Some(icons[0])
    } else {
        None
    }
}

#[cfg(not(windows))]
fn executable_icon_png(_path: &str) -> PowerResult<Vec<u8>> {
    Err(PowerError::NotSupported("executable icon extraction"))
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
struct IconHandle(windows::Win32::UI::WindowsAndMessaging::HICON);

#[cfg(windows)]
impl Drop for IconHandle {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyIcon(self.0) };
        }
    }
}

#[cfg(windows)]
struct BitmapHandle(windows::Win32::Graphics::Gdi::HBITMAP);

#[cfg(windows)]
impl Drop for BitmapHandle {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(
                    windows::Win32::Graphics::Gdi::HGDIOBJ(self.0 .0),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_data_url_adds_png_prefix_and_base64_payload() {
        assert_eq!(png_data_url(&[1, 2, 3]), "data:image/png;base64,AQID");
    }

    #[cfg(windows)]
    #[test]
    fn extracts_icon_from_system_executable() {
        let path = std::env::var("WINDIR")
            .map(|windir| format!("{windir}\\System32\\notepad.exe"))
            .expect("WINDIR");

        let data_url = png_data_url_from_executable(&path).expect("icon data url");

        assert!(data_url.starts_with("data:image/png;base64,"));
        assert!(data_url.len() > 200);
    }
}
