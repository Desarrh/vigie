//! Description lisible d'un exe (« Google Chrome », « Node.js JavaScript Runtime »…) lue dans ses ressources.
use crate::sys::wide;
use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};

pub fn description(path: &str) -> Option<String> {
    let wp = wide(path);
    let size = unsafe { GetFileVersionInfoSizeW(wp.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(wp.as_ptr(), 0, size, buf.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let query = |q: &str| -> Option<(*const u8, u32)> {
        let mut p: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let ok = unsafe { VerQueryValueW(buf.as_ptr().cast(), wide(q).as_ptr(), &mut p, &mut len) };
        (ok != 0 && !p.is_null() && len > 0).then_some((p as *const u8, len))
    };
    let mut langs: Vec<(u16, u16)> = Vec::new();
    if let Some((p, len)) = query(r"\VarFileInfo\Translation") {
        let n = len as usize / 4;
        for i in 0..n {
            let l = unsafe { std::ptr::read_unaligned(p.add(i * 4) as *const u16) };
            let c = unsafe { std::ptr::read_unaligned(p.add(i * 4 + 2) as *const u16) };
            langs.push((l, c));
        }
    }
    langs.extend([(0x040C, 0x04B0), (0x0409, 0x04B0), (0x0409, 0x04E4)]);
    for (l, c) in langs {
        if let Some((p, len)) = query(&format!(r"\StringFileInfo\{l:04x}{c:04x}\FileDescription")) {
            let s = unsafe { std::slice::from_raw_parts(p as *const u16, len as usize) };
            let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
            let d = String::from_utf16_lossy(&s[..n]).trim().to_string();
            if !d.is_empty() {
                return Some(d);
            }
        }
    }
    None
}
