//! Renders numeric tray icons with anti-aliased, transparent text.

use std::mem::size_of;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, GdiFlush, SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER,
    DT_NOCLIP, DT_SINGLELINE, DT_VCENTER, FW_BOLD, HGDIOBJ, OUT_TT_PRECIS, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

pub fn parse_color(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    (p(0), p(2), p(4))
}

/// Draws `text` centered in a `size`x`size` icon. GDI cannot write alpha, so the text is drawn
/// white-on-black and the luminance becomes the alpha channel of the final colored pixel.
pub fn render(text: &str, color: (u8, u8, u8), font: &str, size: i32) -> Option<HICON> {
    unsafe {
        let hdc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old_bmp = SelectObject(hdc, HGDIOBJ(dib.0));

        let chars = text.chars().count() as i32;
        let (height, width) = match chars {
            0..=2 => (size + size / 8, 0),
            _ => (size - size / 10, size * 3 / 10),
        };
        let face: Vec<u16> = font.encode_utf16().chain(std::iter::once(0)).collect();
        let hfont = CreateFontW(
            -height,
            width,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            PCWSTR(face.as_ptr()),
        );
        let old_font = SelectObject(hdc, HGDIOBJ(hfont.0));
        SetTextColor(hdc, COLORREF(0x00FF_FFFF));
        SetBkMode(hdc, TRANSPARENT);
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rc = RECT { left: -size, top: 0, right: size * 2, bottom: size };
        DrawTextW(hdc, &mut wide, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);
        let _ = GdiFlush();

        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        let (r, g, b) = color;
        for px in pixels.iter_mut() {
            let lum = (*px & 0xFF).max((*px >> 8) & 0xFF).max((*px >> 16) & 0xFF);
            *px = (lum << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        }

        SelectObject(hdc, old_font);
        SelectObject(hdc, old_bmp);
        let _ = DeleteObject(HGDIOBJ(hfont.0));

        let mask_bits = vec![0u8; ((size + 15) / 16 * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: dib,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(hdc);
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#FFD60A"), (0xFF, 0xD6, 0x0A));
        assert_eq!(parse_color("30d158"), (0x30, 0xD1, 0x58));
    }

    #[test]
    fn renders_icons() {
        for t in ["5", "42", "100", "–"] {
            let icon = render(t, (255, 200, 0), "Segoe UI", 32).expect("icon");
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon);
            }
        }
    }
}
