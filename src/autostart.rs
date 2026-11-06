//! Launch at sign-in through the per-user Run key. No administrator rights are needed.

use windows::Win32::System::Registry::*;
use windows::core::{HSTRING, w};

/// Writes or removes the Run entry. Re-running it keeps the path right if the exe has moved.
pub fn sync(enabled: bool) {
    unsafe {
        let mut key = HKEY::default();
        let opened = RegCreateKeyW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            &mut key,
        );
        if opened.is_err() {
            return;
        }
        if enabled {
            if let Ok(exe) = std::env::current_exe() {
                let command = format!("\"{}\" --background", exe.display());
                let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
                let bytes = std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
                let _ = RegSetValueExW(key, &HSTRING::from("RSnap"), None, REG_SZ, Some(bytes));
            }
        } else {
            let _ = RegDeleteValueW(key, &HSTRING::from("RSnap"));
        }
        let _ = RegCloseKey(key);
    }
}
