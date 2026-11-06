//! Steps shared by the overlay, toast and editor.

use crate::config;
use crate::imaging::{self, Image};
use crate::model::Object;
use crate::toast;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::core::*;

/// A capture and its annotations, kept editable so the editor can reopen it from a toast.
#[derive(Clone)]
pub struct Session {
    pub base: Rc<Image>,
    pub objects: Vec<Object>,
    pub composed: Rc<Image>,
}

impl Session {
    pub fn new(image: Image) -> Self {
        let base = Rc::new(image);
        Self {
            composed: base.clone(),
            base,
            objects: Vec::new(),
        }
    }
}

thread_local! {
    /// The session behind RSnap's latest clipboard write, so its annotations stay editable.
    static COPIED: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Clipboard write followed by the card toast.
pub fn copy_and_notify(session: Session) {
    match imaging::copy_image(&session.composed) {
        Ok(()) => {
            COPIED.with(|c| *c.borrow_mut() = Some(session.clone()));
            toast::show_card(session, "已複製到剪貼簿".to_string());
        }
        Err(_) => toast::show_message("無法寫入剪貼簿".to_string(), None),
    }
}

/// Shows the card toast for whatever image is on the clipboard right now, which another app
/// may have replaced. An image RSnap put there reopens with its annotations still editable.
pub fn preview_clipboard() {
    let Some(image) = imaging::read_clipboard_image() else {
        toast::show_message("剪貼簿裡沒有圖片".to_string(), None);
        return;
    };
    let copied = COPIED.with(|c| c.borrow().clone());
    let session = copied
        .filter(|s| *s.composed == image)
        .unwrap_or_else(|| Session::new(image));
    toast::show_card(session, "目前的剪貼簿圖片".to_string());
}

/// The configured folder, or the user's Downloads folder by default.
pub fn save_dir() -> PathBuf {
    config::get().save_dir.unwrap_or_else(downloads_dir)
}

fn downloads_dir() -> PathBuf {
    unsafe {
        if let Ok(path) = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None) {
            let text = path.to_string().unwrap_or_default();
            CoTaskMemFree(Some(path.0 as *const _));
            if !text.is_empty() {
                return PathBuf::from(text);
            }
        }
    }
    std::env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Saves to the configured folder. If that fails, asks for another folder and retries once.
/// Returns the saved path, or None when the user cancelled or the retry failed too.
pub fn save_with_recovery(image: &Image, owner: Option<HWND>) -> Option<PathBuf> {
    let format = config::get().format;
    if let Ok(path) = imaging::save(image, &save_dir(), format) {
        return Some(path);
    }
    let chosen = pick_folder(owner, Some(&save_dir()))?;
    match imaging::save(image, &chosen, format) {
        Ok(path) => {
            config::update(|c| c.save_dir = Some(chosen));
            Some(path)
        }
        Err(_) => {
            toast::show_message("儲存失敗，請確認資料夾可以寫入".to_string(), None);
            None
        }
    }
}

pub fn pick_folder(owner: Option<HWND>, start: Option<&Path>) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        let _ = dialog.SetTitle(w!("選擇儲存資料夾"));
        if let Some(start) = start.filter(|p| p.is_dir()) {
            let wide = HSTRING::from(start.as_os_str());
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&wide, None) {
                let _ = dialog.SetFolder(&item);
            }
        }
        dialog.Show(owner).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        text.map(PathBuf::from)
    }
}
