//! Pixel buffers: screen capture, encoding, saving and the clipboard.

use crate::config::ImageFormat;
use crate::model::Rgb;
use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::codecs::webp::WebPEncoder;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_BITMAP, CF_DIB, CF_UNICODETEXT};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, Result, w};

/// Top-down BGRA pixels with every alpha byte at 255.
#[derive(Clone, PartialEq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: i32, y: i32) -> Rgb {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return Rgb::new(0, 0, 0);
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        Rgb::new(self.bgra[i + 2], self.bgra[i + 1], self.bgra[i])
    }

    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Image {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let start = (row as usize * self.w as usize + x as usize) * 4;
            bgra.extend_from_slice(&self.bgra[start..start + w as usize * 4]);
        }
        Image { w, h, bgra }
    }

    fn to_rgb(&self) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(self.w as usize * self.h as usize * 3);
        for px in self.bgra.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        rgb
    }
}

/// The whole virtual desktop, plus its top-left corner in screen coordinates.
pub fn capture_virtual_screen() -> Result<(Image, RECT)> {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let bounds = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };

        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let previous = SelectObject(mem, bitmap.into());
        let copied = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
        SelectObject(mem, previous);
        let image = bitmap_to_image(bitmap);

        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        copied?;
        Ok((image.ok_or_else(Error::from_thread)?, bounds))
    }
}

/// Reads a GDI bitmap that is not currently selected into a device context.
unsafe fn bitmap_to_image(bitmap: HBITMAP) -> Option<Image> {
    unsafe {
        let mut header = BITMAP::default();
        let filled = GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some((&raw mut header).cast()),
        );
        if filled == 0 {
            return None;
        }
        let (w, h) = (header.bmWidth, header.bmHeight);
        let mut bgra = vec![0u8; w as usize * h as usize * 4];
        let mut info = bitmap_info(w, -h);
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            h as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        for px in bgra.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        Some(Image {
            w: w as u32,
            h: h as u32,
            bgra,
        })
    }
}

/// The image on the clipboard, or None if it holds no image. Windows converts any bitmap
/// format another app published to CF_BITMAP on request, so one reader covers them all.
pub fn read_clipboard_image() -> Option<Image> {
    unsafe {
        open_clipboard().ok()?;
        let image = GetClipboardData(CF_BITMAP.0 as u32)
            .ok()
            .and_then(|handle| bitmap_to_image(HBITMAP(handle.0)));
        let _ = CloseClipboard();
        image
    }
}

fn bitmap_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn encode(image: &Image, format: ImageFormat) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let rgb = image.to_rgb();
    let color = image::ExtendedColorType::Rgb8;
    let result = match format {
        ImageFormat::Png => {
            PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
                .write_image(&rgb, image.w, image.h, color)
        }
        ImageFormat::Jpg => {
            JpegEncoder::new_with_quality(&mut out, 92).write_image(&rgb, image.w, image.h, color)
        }
        ImageFormat::Webp => {
            WebPEncoder::new_lossless(&mut out).write_image(&rgb, image.w, image.h, color)
        }
    };
    result.map_err(|e| e.to_string())?;
    Ok(out)
}

fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// `Screenshot_<stamp>.<ext>`, with `_1`, `_2`... added until the name is free.
pub fn unique_path(dir: &Path, stamp: &str, extension: &str) -> PathBuf {
    let first = dir.join(format!("Screenshot_{stamp}.{extension}"));
    if !first.exists() {
        return first;
    }
    (1u32..)
        .map(|n| dir.join(format!("Screenshot_{stamp}_{n}.{extension}")))
        .find(|p| !p.exists())
        .expect("ran out of u32 file names")
}

pub fn save(
    image: &Image,
    dir: &Path,
    format: ImageFormat,
) -> std::result::Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let bytes = encode(image, format)?;
    let path = unique_path(dir, &timestamp(), format.extension());
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Other apps can hold the clipboard open for a moment, so retry before giving up.
fn open_clipboard() -> Result<()> {
    let mut attempt = 0;
    loop {
        match unsafe { OpenClipboard(None) } {
            Ok(()) => return Ok(()),
            Err(e) if attempt >= 20 => return Err(e),
            Err(_) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
}

fn global_copy(parts: &[&[u8]]) -> Result<HGLOBAL> {
    unsafe {
        let size = parts.iter().map(|p| p.len()).sum();
        let handle = GlobalAlloc(GMEM_MOVEABLE, size)?;
        let mut target = GlobalLock(handle) as *mut u8;
        if target.is_null() {
            let error = Error::from_thread();
            let _ = GlobalFree(Some(handle));
            return Err(error);
        }
        for part in parts {
            std::ptr::copy_nonoverlapping(part.as_ptr(), target, part.len());
            target = target.add(part.len());
        }
        let _ = GlobalUnlock(handle);
        Ok(handle)
    }
}

/// Ownership of the memory passes to the clipboard only when SetClipboardData succeeds.
unsafe fn set_data(format: u32, data: HGLOBAL) -> Result<()> {
    unsafe {
        if let Err(e) = SetClipboardData(format, Some(HANDLE(data.0))) {
            let _ = GlobalFree(Some(data));
            return Err(e);
        }
        Ok(())
    }
}

/// Publishes CF_DIB for general compatibility and PNG for apps that prefer it.
pub fn copy_image(image: &Image) -> Result<()> {
    let header = bitmap_info(image.w as i32, image.h as i32).bmiHeader;
    let header_bytes = unsafe {
        std::slice::from_raw_parts(
            (&raw const header).cast::<u8>(),
            size_of::<BITMAPINFOHEADER>(),
        )
    };
    let mut bottom_up = Vec::with_capacity(image.bgra.len());
    for row in image.bgra.chunks_exact(image.w as usize * 4).rev() {
        bottom_up.extend_from_slice(row);
    }
    let dib = global_copy(&[header_bytes, &bottom_up])?;
    let png = encode(image, ImageFormat::Png)
        .ok()
        .and_then(|bytes| global_copy(&[&bytes]).ok());

    unsafe {
        open_clipboard()?;
        let result = (|| {
            EmptyClipboard()?;
            set_data(CF_DIB.0 as u32, dib)?;
            if let Some(png) = png {
                set_data(RegisterClipboardFormatW(w!("PNG")), png)?;
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

pub fn copy_text(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
    let data = global_copy(&[bytes])?;
    unsafe {
        open_clipboard()?;
        let result = (|| {
            EmptyClipboard()?;
            set_data(CF_UNICODETEXT.0 as u32, data)
        })();
        let _ = CloseClipboard();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Image {
        // 2x2: blue, green / red, white.
        Image {
            w: 2,
            h: 2,
            bgra: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rsnap-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pixel_reads_channels_in_rgb_order() {
        let image = sample();
        assert_eq!(image.pixel(0, 0), Rgb::new(0, 0, 255));
        assert_eq!(image.pixel(0, 1), Rgb::new(255, 0, 0));
        assert_eq!(image.pixel(9, 9), Rgb::new(0, 0, 0));
    }

    #[test]
    fn crop_takes_the_right_row_and_column() {
        let cropped = sample().crop(1, 1, 1, 1);
        assert_eq!((cropped.w, cropped.h), (1, 1));
        assert_eq!(cropped.pixel(0, 0), Rgb::new(255, 255, 255));
    }

    #[test]
    fn unique_path_appends_a_counter_on_collision() {
        let dir = temp_dir("names");
        let first = unique_path(&dir, "S", "png");
        assert_eq!(first.file_name().unwrap(), "Screenshot_S.png");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "S", "png");
        assert_eq!(second.file_name().unwrap(), "Screenshot_S_1.png");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_format_writes_a_file_with_the_right_signature() {
        let dir = temp_dir("formats");
        for (format, magic) in [
            (ImageFormat::Png, &b"\x89PNG"[..]),
            (ImageFormat::Jpg, &b"\xFF\xD8\xFF"[..]),
            (ImageFormat::Webp, &b"RIFF"[..]),
        ] {
            let path = save(&sample(), &dir, format).unwrap();
            assert_eq!(path.extension().unwrap(), format.extension());
            assert!(std::fs::read(path).unwrap().starts_with(magic));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_creates_a_missing_folder() {
        let dir = temp_dir("nested").join("a").join("b");
        assert!(save(&sample(), &dir, ImageFormat::Png).unwrap().exists());
    }
}
