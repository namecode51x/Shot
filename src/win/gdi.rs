//! Small RAII guards around GDI objects so handles are released on success,
//! cancellation and error paths alike. Each guard owns exactly one handle and
//! deletes it in `Drop`.

use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};

/// `CreateCompatibleDC(None)` memory DC; `DeleteDC` on drop.
pub struct MemDc(pub HDC);

impl MemDc {
    pub fn new() -> Option<MemDc> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            (!dc.is_invalid()).then_some(MemDc(dc))
        }
    }
}

impl Drop for MemDc {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// An owned GDI object deleted on drop (`DeleteObject`).
pub struct OwnedGdi<T: Into<HGDIOBJ> + Copy>(pub T);

impl<T: Into<HGDIOBJ> + Copy> OwnedGdi<T> {
    pub fn new(obj: T, invalid: bool) -> Option<Self> {
        (!invalid).then_some(OwnedGdi(obj))
    }
    pub fn as_raw(&self) -> T {
        self.0
    }
}

impl OwnedGdi<HBITMAP> {
    pub fn from_bitmap(b: HBITMAP) -> Option<Self> {
        OwnedGdi::new(b, b.is_invalid())
    }
}

impl<T: Into<HGDIOBJ> + Copy> Drop for OwnedGdi<T> {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

/// Restores the previously selected object on drop, then deletes ours.
/// Order matters: an object must not be deleted while selected.
pub struct SelectedObject {
    dc: HDC,
    prev: HGDIOBJ,
    obj: HGDIOBJ,
}

impl SelectedObject {
    /// # Safety
    /// `dc` must stay valid for the lifetime of the guard; `obj` must be owned
    /// by the caller and not selected into any other DC.
    pub unsafe fn select(dc: HDC, obj: HGDIOBJ) -> Option<SelectedObject> {
        let prev = unsafe { SelectObject(dc, obj) };
        (!prev.is_invalid()).then_some(SelectedObject { dc, prev, obj })
    }

    pub fn prev(&self) -> HGDIOBJ {
        self.prev
    }
}

impl Drop for SelectedObject {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.dc, self.prev);
            let _ = DeleteObject(self.obj);
        }
    }
}

/// Screen/desktop DC (unowned, released with `ReleaseDC`).
pub struct ScreenDc {
    hwnd: Option<HWND>,
    hdc: HDC,
}

impl ScreenDc {
    pub fn desktop() -> Option<ScreenDc> {
        unsafe {
            let hdc = windows::Win32::Graphics::Gdi::GetDC(None);
            (!hdc.is_invalid()).then_some(ScreenDc { hwnd: None, hdc })
        }
    }
    pub fn hwnd(&self) -> Option<HWND> {
        self.hwnd
    }
    pub fn hdc(&self) -> HDC {
        self.hdc
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseDC(self.hwnd, self.hdc);
        }
    }
}

/// Moveable global memory block for clipboard payloads.
/// The block is freed on drop only if it was never handed to the clipboard
/// (which takes ownership of `HGLOBAL`s given to `SetClipboardData`).
pub struct GlobalMem {
    h: HGLOBAL,
    taken: bool,
}

impl GlobalMem {
    pub fn alloc(bytes: usize) -> Option<GlobalMem> {
        unsafe {
            // GMEM_MOVEABLE (0x0002) | GMEM_ZEROINIT (0x0040)
            let h = GlobalAlloc(
                windows::Win32::System::Memory::GLOBAL_ALLOC_FLAGS(0x0042),
                bytes,
            )
            .ok()?;
            Some(GlobalMem { h, taken: false })
        }
    }

    pub fn size(&self) -> usize {
        unsafe { GlobalSize(self.h) }
    }

    /// Write `data` into the block (lock, copy, unlock).
    pub fn write(&mut self, data: &[u8]) -> bool {
        if self.size() < data.len() {
            return false;
        }
        unsafe {
            let p = GlobalLock(self.h) as *mut u8;
            if p.is_null() {
                return false;
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
            let _ = GlobalUnlock(self.h);
            true
        }
    }

    /// Mark the block as owned by the clipboard (no free on drop).
    pub fn leak(&mut self) {
        self.taken = true;
    }

    pub fn raw(&self) -> HGLOBAL {
        self.h
    }
}

impl Drop for GlobalMem {
    fn drop(&mut self) {
        if self.taken {
            return;
        }
        unsafe {
            let _ = GlobalFree(Some(self.h));
        }
    }
}

/// Close a HANDLE (event, mapping...) on drop.
pub struct CloseHandleGuard(pub HANDLE);

impl Drop for CloseHandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Module instance of the current EXE.
pub fn this_module() -> HINSTANCE {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    unsafe { GetModuleHandleW(None).unwrap_or_default().into() }
}

// ---------------------------------------------------------------------------
// Small painting helpers shared by the custom-drawn windows.
// ---------------------------------------------------------------------------

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DRAW_TEXT_FORMAT, DT_NOPREFIX, DrawTextW, FillRect, FrameRect, RoundRect,
    SetBkMode, SetTextColor, TRANSPARENT,
};

/// Fill `r` with a solid color given as a palette constant (`0x00BBGGRR`).
pub fn fill_solid(hdc: HDC, r: &RECT, color: u32) {
    unsafe {
        let br = CreateSolidBrush(crate::win::dark::colorref_u32(color));
        FillRect(hdc, r, br);
        let _ = DeleteObject(br.into());
    }
}

/// Draw a 1px frame around `r` in the given palette color.
pub fn frame_solid(hdc: HDC, r: &RECT, color: u32) {
    unsafe {
        let br = CreateSolidBrush(crate::win::dark::colorref_u32(color));
        FrameRect(hdc, r, br);
        let _ = DeleteObject(br.into());
    }
}

/// Rounded-rectangle face with a 1px border in a single GDI pass
/// (`RoundRect` with a solid brush + 1px pen). Pass `border == fill` for a
/// borderless face. Corner radius comes from `theme::RADIUS`.
pub fn round_face(hdc: HDC, r: &RECT, fill: u32, border: u32) {
    unsafe {
        use windows::Win32::Graphics::Gdi::{CreatePen, PS_SOLID};
        let br = CreateSolidBrush(crate::win::dark::colorref_u32(fill));
        let pen = CreatePen(PS_SOLID, 1, crate::win::dark::colorref_u32(border));
        let prev_b = SelectObject(hdc, br.into());
        let prev_p = SelectObject(hdc, pen.into());
        let rad = crate::theme::RADIUS;
        let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, rad, rad);
        SelectObject(hdc, prev_p);
        SelectObject(hdc, prev_b);
        let _ = DeleteObject(pen.into());
        let _ = DeleteObject(br.into());
    }
}

/// Draw a single text line with a transparent background. The buffer passed to
/// GDI includes the trailing NUL (zero-width, harmless) — same convention as
/// [`crate::text`].
pub fn draw_label(hdc: HDC, r: RECT, text: &str, color: u32, format: DRAW_TEXT_FORMAT) {
    let mut buf = crate::win::wide(text);
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, crate::win::dark::colorref_u32(color));
        let mut rc = r;
        DrawTextW(hdc, &mut buf, &mut rc, format | DT_NOPREFIX);
    }
}

/// Stock system cursors, loaded once per process. `WM_SETCURSOR` fires on
/// every mousemove and each `LoadCursorW` is a round-trip out of the hot
/// path — so the nine cursors the app ever shows are resolved a single
/// time. Unknown ids keep the legacy load-or-default behavior.
pub fn system_cursor(idc: windows::core::PCWSTR) -> windows::Win32::UI::WindowsAndMessaging::HCURSOR {
    use windows::Win32::UI::WindowsAndMessaging::{
        HCURSOR, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS,
        IDC_SIZENWSE, IDC_SIZEWE, LoadCursorW,
    };

    fn load(idc: windows::core::PCWSTR) -> *mut std::ffi::c_void {
        unsafe { LoadCursorW(None, idc).map(|c| c.0).unwrap_or(std::ptr::null_mut()) }
    }

    /// A stock system cursor handle. System cursors are process-global,
    /// immutable and never freed, so sharing the raw value is sound.
    #[derive(Clone, Copy)]
    struct SharedCursor(*mut std::ffi::c_void);
    // SAFETY: see above — never dereferenced, only handed back to Win32.
    unsafe impl Send for SharedCursor {}
    unsafe impl Sync for SharedCursor {}

    static CURSORS: std::sync::OnceLock<[SharedCursor; 9]> = std::sync::OnceLock::new();
    let c = CURSORS.get_or_init(|| {
        [
            SharedCursor(load(IDC_ARROW)),
            SharedCursor(load(IDC_CROSS)),
            SharedCursor(load(IDC_SIZEALL)),
            SharedCursor(load(IDC_SIZENWSE)),
            SharedCursor(load(IDC_SIZENESW)),
            SharedCursor(load(IDC_SIZENS)),
            SharedCursor(load(IDC_SIZEWE)),
            SharedCursor(load(IDC_IBEAM)),
            SharedCursor(load(IDC_HAND)),
        ]
    });
    let raw = if idc == IDC_ARROW {
        c[0].0
    } else if idc == IDC_CROSS {
        c[1].0
    } else if idc == IDC_SIZEALL {
        c[2].0
    } else if idc == IDC_SIZENWSE {
        c[3].0
    } else if idc == IDC_SIZENESW {
        c[4].0
    } else if idc == IDC_SIZENS {
        c[5].0
    } else if idc == IDC_SIZEWE {
        c[6].0
    } else if idc == IDC_IBEAM {
        c[7].0
    } else if idc == IDC_HAND {
        c[8].0
    } else {
        std::ptr::null_mut()
    };
    if !raw.is_null() {
        HCURSOR(raw)
    } else {
        unsafe {
            LoadCursorW(None, idc)
                .or_else(|_| LoadCursorW(None, IDC_CROSS))
                .unwrap_or_default()
        }
    }
}
