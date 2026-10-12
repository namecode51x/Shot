//! Native dialogs and message boxes: Save As (IFileSaveDialog), folder picker
//! (IFileOpenDialog with FOS_PICKFOLDERS) and small message boxes.
//! All dialogs are invoked synchronously from a UI action and are owned by the
//! calling window; no nested application loops of our own are created.

use crate::config::OutputFormat;
use crate::error::{Error, Result};
use crate::win::wide;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IBindCtx,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
    FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog,
    KF_FLAG_DEFAULT, SHCreateItemFromParsingName, SHGetKnownFolderPath, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_TASKMODAL, MB_YESNO,
    MESSAGEBOX_RESULT, MESSAGEBOX_STYLE, MessageBoxW,
};
use windows::core::HRESULT;

pub fn error_box(owner: Option<HWND>, title: &str, text: &str) {
    message_box(owner, title, text, MB_OK | MB_ICONERROR | MB_TASKMODAL);
}

pub fn warn_box(owner: Option<HWND>, title: &str, text: &str) {
    message_box(owner, title, text, MB_OK | MB_ICONWARNING | MB_TASKMODAL);
}

pub fn info_box(owner: Option<HWND>, title: &str, text: &str) {
    message_box(owner, title, text, MB_OK | MB_TASKMODAL);
}

/// Yes/No confirmation. `true` when the user chose Yes.
pub fn confirm(owner: Option<HWND>, title: &str, text: &str) -> bool {
    let r = message_box(
        owner,
        title,
        text,
        MB_YESNO | MB_ICONQUESTION | MB_TASKMODAL,
    );
    r.0 == IDYES.0
}

fn message_box(
    owner: Option<HWND>,
    title: &str,
    text: &str,
    style: MESSAGEBOX_STYLE,
) -> MESSAGEBOX_RESULT {
    let t = wide(title);
    let x = wide(text);
    unsafe {
        MessageBoxW(
            owner,
            windows::core::PCWSTR(x.as_ptr()),
            windows::core::PCWSTR(t.as_ptr()),
            style,
        )
    }
}

/// The Pictures known folder, or `None` when it cannot be resolved.
pub fn pictures_folder() -> Option<PathBuf> {
    unsafe {
        match SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None) {
            Ok(p) => {
                let s = crate::win::from_wide(p.0);
                CoTaskMemFree(Some(p.0 as *const _));
                (!s.is_empty()).then(|| PathBuf::from(s))
            }
            Err(_) => None,
        }
    }
}

/// Folder used when no explicit save folder is configured:
/// settings value → Pictures → `%USERPROFILE%\Pictures` → config dir.
pub fn resolve_save_dir(configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(configured.trim());
    }
    if let Some(p) = pictures_folder() {
        return p;
    }
    if let Some(up) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(up).join("Pictures");
        return p;
    }
    crate::config::config_dir()
}

fn set_initial_dir(dlg: &IFileDialog, dir: &Path) {
    let w = wide(&dir.to_string_lossy());
    if w.len() < 2 {
        return;
    }
    let item: std::result::Result<windows::Win32::UI::Shell::IShellItem, _> = unsafe {
        SHCreateItemFromParsingName(windows::core::PCWSTR(w.as_ptr()), None::<&IBindCtx>)
    };
    if let Ok(item) = item {
        let _ = unsafe { dlg.SetFolder(&item) };
    }
}

/// Save-As dialog. Returns `None` when the user cancelled, otherwise the full
/// path the user confirmed (the file does not exist yet at this point).
pub fn save_image(
    owner: Option<HWND>,
    initial_dir: Option<&Path>,
    default_name: &str,
    format: OutputFormat,
) -> Result<Option<PathBuf>> {
    unsafe {
        let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| Error::Win32(e).with_context("CoCreateInstance(IFileSaveDialog)"))?;

        let n_png = wide("PNG image");
        let s_png = wide("*.png");
        let n_jpg = wide("JPEG image");
        let s_jpg = wide("*.jpg;*.jpeg");
        let n_bmp = wide("BMP image");
        let s_bmp = wide("*.bmp");
        let specs = [
            COMDLG_FILTERSPEC {
                pszName: windows::core::PCWSTR(n_png.as_ptr()),
                pszSpec: windows::core::PCWSTR(s_png.as_ptr()),
            },
            COMDLG_FILTERSPEC {
                pszName: windows::core::PCWSTR(n_jpg.as_ptr()),
                pszSpec: windows::core::PCWSTR(s_jpg.as_ptr()),
            },
            COMDLG_FILTERSPEC {
                pszName: windows::core::PCWSTR(n_bmp.as_ptr()),
                pszSpec: windows::core::PCWSTR(s_bmp.as_ptr()),
            },
        ];
        dlg.SetFileTypes(&specs)
            .map_err(|e| Error::Win32(e).with_context("IFileDialog::SetFileTypes"))?;
        let _ = dlg.SetFileTypeIndex(match format {
            OutputFormat::Png => 1,
            OutputFormat::Jpeg => 2,
            OutputFormat::Bmp => 3,
        });
        let ext = wide(format.extension());
        dlg.SetDefaultExtension(windows::core::PCWSTR(ext.as_ptr()))
            .map_err(|e| Error::Win32(e).with_context("IFileDialog::SetDefaultExtension"))?;
        let name = wide(default_name);
        dlg.SetFileName(windows::core::PCWSTR(name.as_ptr()))
            .map_err(|e| Error::Win32(e).with_context("IFileDialog::SetFileName"))?;
        dlg.SetOptions(FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_OVERWRITEPROMPT)
            .map_err(|e| Error::Win32(e).with_context("IFileDialog::SetOptions"))?;
        if let Some(dir) = initial_dir {
            set_initial_dir(&dlg, dir);
        }

        match dlg.Show(owner) {
            Ok(()) => {}
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
            Err(e) => return Err(Error::Win32(e).with_context("IFileDialog::Show")),
        }

        let item = dlg
            .GetResult()
            .map_err(|e| Error::Win32(e).with_context("IFileDialog::GetResult"))?;
        let pw = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| Error::Win32(e).with_context("IShellItem::GetDisplayName"))?;
        let path = PathBuf::from(crate::win::from_wide(pw.0));
        CoTaskMemFree(Some(pw.0 as *const _));
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        Ok(Some(path))
    }
}

/// Folder picker. `None` when cancelled.
pub fn pick_folder(owner: Option<HWND>, initial_dir: Option<&Path>) -> Result<Option<PathBuf>> {
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| Error::Win32(e).with_context("CoCreateInstance(IFileOpenDialog)"))?;
        dlg.SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            .map_err(|e| Error::Win32(e).with_context("IFileDialog::SetOptions"))?;
        if let Some(dir) = initial_dir {
            set_initial_dir(&dlg, dir);
        }

        match dlg.Show(owner) {
            Ok(()) => {}
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
            Err(e) => return Err(Error::Win32(e).with_context("IFileDialog::Show")),
        }

        let item = dlg
            .GetResult()
            .map_err(|e| Error::Win32(e).with_context("IFileDialog::GetResult"))?;
        let pw = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| Error::Win32(e).with_context("IShellItem::GetDisplayName"))?;
        let path = PathBuf::from(crate::win::from_wide(pw.0));
        CoTaskMemFree(Some(pw.0 as *const _));
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        Ok(Some(path))
    }
}
