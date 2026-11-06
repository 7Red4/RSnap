//! The global capture hotkey.

use crate::config::{self, Hotkey};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

pub const ID: i32 = 1;

thread_local! {
    static OWNER: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static STATUS: RefCell<String> = const { RefCell::new(String::new()) };
}

fn register(key: Hotkey) -> bool {
    let owner = OWNER.with(Cell::get);
    unsafe {
        RegisterHotKey(
            Some(owner),
            ID,
            HOT_KEY_MODIFIERS(key.modifiers) | MOD_NOREPEAT,
            key.vk,
        )
        .is_ok()
    }
}

/// Registers the saved hotkey at startup. Returns false if another app already holds it.
pub fn init(owner: HWND) -> bool {
    OWNER.with(|o| o.set(owner));
    let key = config::get().hotkey;
    let ok = register(key);
    set_status(if ok {
        format!("{} 已啟用", describe(key))
    } else {
        format!("{} 已被其他程式佔用，請換一組", describe(key))
    });
    ok
}

/// Switches to a new hotkey and saves it. On failure the previous hotkey stays active.
pub fn apply(key: Hotkey) {
    let previous = config::get().hotkey;
    let owner = OWNER.with(Cell::get);
    unsafe {
        let _ = UnregisterHotKey(Some(owner), ID);
    }
    if register(key) {
        config::update(|c| c.hotkey = key);
        set_status(format!("{} 已啟用", describe(key)));
    } else {
        let restored = register(previous);
        set_status(format!(
            "{} 已被其他程式佔用{}",
            describe(key),
            if restored {
                format!("，繼續使用 {}", describe(previous))
            } else {
                String::new()
            }
        ));
    }
}

pub fn status_text() -> String {
    STATUS.with(|s| s.borrow().clone())
}

fn set_status(text: String) {
    STATUS.with(|s| *s.borrow_mut() = text);
}

pub fn describe(key: Hotkey) -> String {
    let mut parts: Vec<String> = Vec::new();
    if key.modifiers & 2 != 0 {
        parts.push("Ctrl".into());
    }
    if key.modifiers & 4 != 0 {
        parts.push("Shift".into());
    }
    if key.modifiers & 1 != 0 {
        parts.push("Alt".into());
    }
    if key.modifiers & 8 != 0 {
        parts.push("Win".into());
    }
    parts.push(key_name(key.vk));
    parts.join(" + ")
}

fn key_name(vk: u32) -> String {
    match vk {
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x2C => "PrintScreen".into(),
        _ => format!("0x{vk:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_default_hotkey() {
        assert_eq!(
            describe(Hotkey {
                modifiers: 2,
                vk: 0x70
            }),
            "Ctrl + F1"
        );
    }

    #[test]
    fn describes_combined_modifiers() {
        assert_eq!(
            describe(Hotkey {
                modifiers: 2 | 4 | 1,
                vk: 0x53
            }),
            "Ctrl + Shift + Alt + S"
        );
    }
}
