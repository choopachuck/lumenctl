//! Renders numeric tray icons the way ClickMonitorDDC draws them.
//!
//! The icon is the taskbar's small-icon square. The font's cell height is
//! 125% of that square and the digits are centered in it, so the empty leading
//! is what gets clipped and the ink fills the tile. A three-digit value is
//! drawn at 27% of that height in width, which is how "100" stays as tall as
//! "90". A face whose name already pins a light weight stays at that weight:
//! GDI will not embolden "ExtraLight", and overstriking it closes the counters.

use std::mem::size_of;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, GdiFlush, SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_BOLD, FW_NORMAL, HDC, HFONT, HGDIOBJ, OUT_TT_PRECIS,
    TRANSPARENT,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, FindWindowW, HICON, ICONINFO, SM_CXSMICON,
};

pub fn parse_color(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    (p(0), p(2), p(4))
}

/// Pixel size Explorer will display the icon at, for the taskbar's current DPI.
pub fn tray_px() -> i32 {
    unsafe {
        let dpi = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())
            .ok()
            .map(|hwnd| GetDpiForWindow(hwnd))
            .filter(|dpi| *dpi >= 96)
            .unwrap_or(96);
        GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16)
    }
}

/// Draws `text` centered in a `size`×`size` icon.
pub fn render(text: &str, color: (u8, u8, u8), font: &str, size: i32) -> Option<HICON> {
    let pixels = raster(text, color, font, size)?;
    hicon(&pixels, size)
}

fn raster(text: &str, color: (u8, u8, u8), font: &str, size: i32) -> Option<Vec<u32>> {
    if size < 8 || text.is_empty() {
        return None;
    }
    let drawn = unsafe {
        let hdc = CreateCompatibleDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let drawn = draw_icon(hdc, text, font, size);
        let _ = DeleteDC(hdc);
        drawn
    }?;
    Some(colorize(&drawn, size, color))
}

#[cfg(test)]
struct Bounds {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Light faces stay thin under `FW_BOLD` when the face name already names the weight.
fn synthetic_bold(font: &str) -> bool {
    let n = font.to_ascii_lowercase();
    n.contains("thin") || n.contains("light") || n.contains("regular") || n.contains("normal")
}

/// White text on black, at the icon's own pixel size.
unsafe fn draw_icon(hdc: HDC, text: &str, font: &str, size: i32) -> Option<Vec<u32>> {
    let face: Vec<u16> = font.encode_utf16().chain(std::iter::once(0)).collect();
    // Cell height, not em height. Positive `CreateFont` height matches ClickMonitor:
    // the cell is 125% of the icon and centering clips the leading, not the digits.
    let height = (size * 125 / 100).max(1);
    // "100" is the only three-digit reading. Width is 27% of the cell height.
    let width = if text.chars().count() >= 3 {
        (height * 270 / 1000).max(1)
    } else {
        0
    };
    let weight = if synthetic_bold(font) {
        FW_NORMAL.0
    } else {
        FW_BOLD.0
    } as i32;

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
    let count = (size * size) as usize;
    std::ptr::write_bytes(bits, 0, count * 4);
    let old_bmp = SelectObject(hdc, HGDIOBJ(dib.0));
    let hfont = make_font(&face, height, width, weight);
    if hfont.is_invalid() {
        SelectObject(hdc, old_bmp);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        return None;
    }
    let old_font = SelectObject(hdc, HGDIOBJ(hfont.0));
    SetTextColor(hdc, COLORREF(0x00FF_FFFF));
    SetBkMode(hdc, TRANSPARENT);
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: size,
        bottom: size,
    };
    DrawTextW(
        hdc,
        &mut wide,
        &mut rc,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
    let _ = GdiFlush();
    let pixels = std::slice::from_raw_parts(bits as *const u32, count).to_vec();
    SelectObject(hdc, old_font);
    SelectObject(hdc, old_bmp);
    let _ = DeleteObject(HGDIOBJ(hfont.0));
    let _ = DeleteObject(HGDIOBJ(dib.0));
    Some(pixels)
}

unsafe fn make_font(face: &[u16], height: i32, width: i32, weight: i32) -> HFONT {
    CreateFontW(
        height,
        width,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_TT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        ANTIALIASED_QUALITY,
        0,
        PCWSTR(face.as_ptr()),
    )
}

/// Coverage of the white drawing becomes the icon color, with premultiplied alpha.
fn colorize(src: &[u32], size: i32, color: (u8, u8, u8)) -> Vec<u32> {
    let (cr, cg, cb) = color;
    src.iter()
        .take((size * size) as usize)
        .map(|p| {
            let a = luminance(*p).min(255) as u8;
            let pr = (cr as u32 * a as u32 + 127) / 255;
            let pg = (cg as u32 * a as u32 + 127) / 255;
            let pb = (cb as u32 * a as u32 + 127) / 255;
            (a as u32) << 24 | pr << 16 | pg << 8 | pb
        })
        .collect()
}

fn luminance(p: u32) -> u32 {
    (p & 0xFF).max((p >> 8) & 0xFF).max((p >> 16) & 0xFF)
}

#[cfg(test)]
fn ink_bounds(px: &[u32], w: i32, h: i32) -> Option<Bounds> {
    let mut x0 = w;
    let mut y0 = h;
    let mut x1 = 0;
    let mut y1 = 0;
    for y in 0..h {
        for x in 0..w {
            if luminance(px[(y * w + x) as usize]) > 24 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some(Bounds {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

fn hicon(pixels: &[u32], size: i32) -> Option<HICON> {
    unsafe {
        let hdc = CreateCompatibleDC(None);
        if hdc.is_invalid() {
            return None;
        }
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
        let Some(dib) = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()
        else {
            let _ = DeleteDC(hdc);
            return None;
        };
        let count = (size * size) as usize;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, count);

        // All-zero AND mask: the color bitmap's alpha is the shape.
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
    fn glyphs_stay_inside_the_icon() {
        for (font, text) in [
            ("Segoe UI", "0"),
            ("Segoe UI", "38"),
            ("Segoe UI", "85"),
            ("Segoe UI", "100"),
            ("Cascadia Mono ExtraLight", "0"),
            ("Cascadia Mono ExtraLight", "90"),
            ("Cascadia Mono ExtraLight", "38"),
            ("Cascadia Mono ExtraLight", "100"),
            ("Cascadia Mono ExtraLight", "–"),
        ] {
            for size in [16, 24, 32] {
                let px = raster(text, (0x32, 0xA8, 0xFF), font, size).expect("raster");
                assert_eq!(px.len(), (size * size) as usize);
                let s = size as usize;
                let alpha = |i: usize| px[i] >> 24;
                let bounds = super::ink_bounds(&px, size, size).expect("ink");
                if text.chars().any(|c| c.is_ascii_digit()) {
                    assert!(
                        bounds.w.max(bounds.h) >= size * 3 / 5,
                        "{font} {text} @{size} ink {}x{} does not fill the icon",
                        bounds.w,
                        bounds.h
                    );
                }
                let dx = (bounds.x - (size - bounds.x - bounds.w)).abs();
                let dy = (bounds.y - (size - bounds.y - bounds.h)).abs();
                assert!(
                    dx <= 3 && dy <= size / 4,
                    "{font} {text} @{size} ink at {},{} {}x{} is off center",
                    bounds.x,
                    bounds.y,
                    bounds.w,
                    bounds.h
                );
                if size == 16 && std::env::var("ICON_DUMP").is_ok() {
                    println!("{font} {text}");
                    for y in 0..size {
                        let row: String = (0..size)
                            .map(|x| {
                                let a = alpha((y as usize) * s + x as usize);
                                if a > 160 {
                                    '#'
                                } else if a > 40 {
                                    '+'
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        println!("{row}");
                    }
                    dump(&px, size, &format!("{font}-{text}"));
                }
            }
        }
    }

    fn dump(px: &[u32], size: i32, name: &str) {
        let zoom = 8i32;
        let out = size * zoom;
        let mut bgra = vec![0u8; (out * out * 4) as usize];
        for y in 0..out {
            for x in 0..out {
                let p = px[((y / zoom) * size + x / zoom) as usize];
                let a = (p >> 24) & 0xFF;
                let pr = (p >> 16) & 0xFF;
                let pg = (p >> 8) & 0xFF;
                let pb = p & 0xFF;
                let bg = 26u32;
                let i = ((y * out + x) * 4) as usize;
                bgra[i] = (pb + bg * (255 - a) / 255) as u8;
                bgra[i + 1] = (pg + bg * (255 - a) / 255) as u8;
                bgra[i + 2] = (pr + bg * (255 - a) / 255) as u8;
                bgra[i + 3] = 255;
            }
        }
        let mut file = Vec::new();
        let header = 54u32;
        let bytes = bgra.len() as u32;
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&(header + bytes).to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&header.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&out.to_le_bytes());
        file.extend_from_slice(&(-out).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&bytes.to_le_bytes());
        file.extend_from_slice(&2835u32.to_le_bytes());
        file.extend_from_slice(&2835u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&bgra);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/icon-samples");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(format!("{name}.bmp")), file).unwrap();
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
