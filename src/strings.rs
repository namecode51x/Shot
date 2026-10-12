//! All user-visible text in one place, so a future localization pass
//! only has to swap this table. Identifiers and comments stay English.

pub struct Strings {
    pub app_name: &'static str,
    pub app_description: &'static str,
    pub tray_capture: &'static str,
    pub tray_settings: &'static str,
    pub tray_about: &'static str,
    pub tray_exit: &'static str,
    pub tray_tooltip: &'static str,

    pub hotkey_conflict_title: &'static str,
    pub hotkey_conflict_msg: &'static str,
    pub hotkey_unavailable_title: &'static str,

    pub os_unsupported_title: &'static str,
    pub os_unsupported_msg: &'static str,
    pub already_running_title: &'static str,
    pub already_running_msg: &'static str,

    pub capture_hint: &'static str,
    pub capture_hint_keys: &'static str,
    pub capture_fail_title: &'static str,
    pub capture_fail_msg: &'static str,
    pub capture_empty_msg: &'static str,
    pub secure_surface_msg: &'static str,

    pub editor_w: &'static str,
    pub tool_pen: &'static str,
    pub tool_line: &'static str,
    pub tool_arrow: &'static str,
    pub tool_rect: &'static str,
    pub tool_box: &'static str,
    pub tool_text: &'static str,
    pub tool_highlight: &'static str,
    pub tool_mosaic: &'static str,
    pub tool_solid: &'static str,
    pub tool_undo: &'static str,
    pub tool_redo: &'static str,
    pub tool_copy: &'static str,
    pub tool_save: &'static str,
    pub tool_close: &'static str,
    pub tool_color: &'static str,
    pub tool_width: &'static str,
    pub tool_zoom_in: &'static str,
    pub tool_zoom_out: &'static str,
    pub tool_zoom_fit: &'static str,
    pub text_prompt: &'static str,
    /// Accessible name for the transient text-tool input box (AT only).
    pub text_input_name: &'static str,

    /// Hover tooltips, rendered in the overlay's status chip (the overlay
    /// uses no floating tooltip windows and no timers).
    pub tip_pen: &'static str,
    pub tip_line: &'static str,
    pub tip_arrow: &'static str,
    pub tip_rect: &'static str,
    pub tip_box: &'static str,
    pub tip_text: &'static str,
    pub tip_mark: &'static str,
    pub tip_mosaic: &'static str,
    pub tip_solid: &'static str,
    pub tip_undo: &'static str,
    pub tip_redo: &'static str,
    pub tip_copy: &'static str,
    pub tip_save: &'static str,
    pub tip_close: &'static str,
    pub tip_width: &'static str,
    /// Color names in [`crate::editor::PALETTE`] order.
    pub color_names: [&'static str; 8],

    pub state_copied: &'static str,
    pub state_saved: &'static str,
    pub state_copy_fail_title: &'static str,
    pub state_copy_fail: &'static str,
    pub state_save_fail_title: &'static str,
    pub state_save_fail: &'static str,

    pub settings_title: &'static str,
    pub settings_group_capture: &'static str,
    pub settings_group_save: &'static str,
    pub settings_group_startup: &'static str,
    pub settings_hotkey: &'static str,
    pub settings_change: &'static str,
    pub settings_capture_now: &'static str,
    pub settings_format: &'static str,
    pub settings_folder: &'static str,
    pub settings_browse: &'static str,
    pub settings_quick_save: &'static str,
    pub settings_quick_save_hint: &'static str,
    pub settings_startup: &'static str,
    pub settings_startup_hint: &'static str,
    pub settings_jpeg_quality: &'static str,
    pub settings_ok: &'static str,
    pub settings_cancel: &'static str,
    pub settings_saved: &'static str,
    pub settings_invalid_hotkey: &'static str,
    pub hotkey_dialog_title: &'static str,
    pub hotkey_dialog_prompt: &'static str,
    pub hotkey_dialog_hint: &'static str,
    pub hotkey_dialog_ok: &'static str,
    pub hotkey_dialog_cancel: &'static str,
    pub hotkey_dialog_bad: &'static str,

    pub about_title: &'static str,
    pub about_version: &'static str,
    pub about_privacy: &'static str,
    pub about_close: &'static str,
    pub about_os: &'static str,

    pub err_capture_init: &'static str,
    pub err_hotkey_register: &'static str,
    pub err_generic: &'static str,
}

pub static EN: Strings = Strings {
    app_name: "NameShot",
    app_description: "Privacy-first regional screenshot tool",
    tray_capture: "Capture",
    tray_settings: "Settings...",
    tray_about: "About",
    tray_exit: "Exit",
    tray_tooltip: "NameShot - Print Screen to capture",

    hotkey_conflict_title: "Capture hotkey unavailable",
    hotkey_conflict_msg: "The hotkey {H} is already registered by Windows or another application.\n\
         NameShot keeps running; use the tray menu to capture, or open Settings to choose another hotkey.",
    hotkey_unavailable_title: "Hotkey not registered",

    os_unsupported_title: "Unsupported Windows version",
    os_unsupported_msg: "NameShot requires Windows 10 or later. This system reports build {BUILD}.",
    already_running_title: "NameShot is already running",
    already_running_msg: "NameShot is already in the notification area. Look for the tray icon.",

    capture_hint: "Drag to select - release to edit",
    capture_hint_keys: "Enter: use   F: full screen   R: restart   Esc: cancel",
    capture_fail_title: "Capture failed",
    capture_fail_msg: "The desktop could not be captured.\n{DETAIL}",
    capture_empty_msg: "The selection is empty.",
    secure_surface_msg: "Part of the screen is protected (secure desktop or DRM) and appears black.",

    editor_w: "NameShot - Editor",
    tool_pen: "Pen",
    tool_line: "Line",
    tool_arrow: "Arrow",
    tool_rect: "Rect",
    tool_box: "Box",
    tool_text: "Text",
    tool_highlight: "Mark",
    tool_mosaic: "Mosaic",
    tool_solid: "Redact",
    tool_undo: "Undo",
    tool_redo: "Redo",
    tool_copy: "Copy",
    tool_save: "Save",
    tool_close: "Close",
    tool_color: "Color",
    tool_width: "Width",
    tool_zoom_in: "Zoom in",
    tool_zoom_out: "Zoom out",
    tool_zoom_fit: "Fit",
    text_prompt: "Type, Enter to place, Esc to cancel",
    text_input_name: "Text input",

    tip_pen: "Freehand drawing",
    tip_line: "Straight line",
    tip_arrow: "Line with arrowhead",
    tip_rect: "Filled rectangle",
    tip_box: "Outlined rectangle",
    tip_text: "Click the image, type, Enter to place",
    tip_mark: "Marker highlight swipe",
    tip_mosaic: "Pixelate a region",
    tip_solid: "Solid block over content",
    tip_undo: "Undo last change (Ctrl+Z)",
    tip_redo: "Redo (Ctrl+Y)",
    tip_copy: "Copy image to clipboard (Ctrl+C)",
    tip_save: "Save image to file (Ctrl+S)",
    tip_close: "Close overlay (Esc)",
    tip_width: "Stroke width",
    color_names: [
        "White", "Black", "Red", "Orange", "Green", "Blue", "Teal", "Purple",
    ],

    state_copied: "Copied to clipboard",
    state_saved: "Saved",
    state_copy_fail_title: "Copy failed",
    state_copy_fail: "The image could not be placed on the clipboard.\n{DETAIL}",
    state_save_fail_title: "Save failed",
    state_save_fail: "The file could not be written.\n{DETAIL}",

    settings_title: "NameShot - Settings",
    settings_group_capture: "Capture & Hotkey",
    settings_group_save: "Save & Format",
    settings_group_startup: "Startup",
    settings_hotkey: "Capture hotkey",
    settings_change: "Change...",
    settings_capture_now: "Capture now",
    settings_format: "Image format",
    settings_folder: "Save folder",
    settings_browse: "Browse...",
    settings_quick_save: "Enable \"Quick save\" tray action (saves to this folder)",
    settings_quick_save_hint: "Off by default. Never overwrites an existing file.",
    settings_startup: "Start NameShot when I sign in",
    settings_startup_hint: "Off by default; enabling it writes one HKCU Run value.",
    settings_jpeg_quality: "JPEG quality",
    settings_ok: "OK",
    settings_cancel: "Cancel",
    settings_saved: "Settings saved",
    settings_invalid_hotkey: "That hotkey is not allowed. Use at least one modifier, or a key like PrintScreen/F5.",
    hotkey_dialog_title: "Change hotkey",
    hotkey_dialog_prompt: "Press the new key combination now",
    hotkey_dialog_hint: "Example: Ctrl+Shift+S   or   PrintScreen",
    hotkey_dialog_ok: "Apply",
    hotkey_dialog_cancel: "Cancel",
    hotkey_dialog_bad: "That combination is not allowed.",

    about_title: "About NameShot",
    about_version: "Version {VERSION}",
    about_privacy: "Local-only: no account, no telemetry, no network access.\n\
                    Screenshots stay on this PC unless you copy or save them.",
    about_close: "Close",
    about_os: "OS: {OS}",

    err_capture_init: "The screen capture could not be initialized.",
    err_hotkey_register: "The capture hotkey could not be registered.",
    err_generic: "Unexpected error.",
};

pub fn s() -> &'static Strings {
    &EN
}

/// Replace the first `{NAME}` placeholder with a value.
pub fn fill(template: &str, name: &str, value: &str) -> String {
    let key = format!("{{{}}}", name);
    match template.find(&key) {
        Some(i) => {
            let mut out = String::with_capacity(template.len() + value.len());
            out.push_str(&template[..i]);
            out.push_str(value);
            out.push_str(&template[i + key.len()..]);
            out
        }
        None => template.to_string(),
    }
}
