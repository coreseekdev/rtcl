//! Tk's system services on Windows, through the Win32 calls Tk itself makes
//! (`tkWinDialog.c`): each dialog blocks in the system's modal loop until
//! the user answers, which is what the Tcl commands promise.

use rtcl_tk_runtime::{ColorDialogRequest, FileDialogKind, FileDialogRequest, MessageBoxRequest};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{COLORREF, HGLOBAL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::UI::Controls::Dialogs::{
    ChooseColorW, GetOpenFileNameW, GetSaveFileNameW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW,
    CHOOSECOLOR_FLAGS, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
    OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS,
    BROWSEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, DialogBoxIndirectParamW, EndDialog, GetSystemMetrics, MessageBoxW,
    PeekMessageW, PostQuitMessage, SendMessageW, TranslateMessage, DLGPROC, DLGTEMPLATE,
    DS_CENTER, DS_MODALFRAME, DS_SETFONT,
    MB_ABORTRETRYIGNORE, MB_DEFBUTTON1, MB_DEFBUTTON2, MB_DEFBUTTON3, MB_ICONERROR,
    MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_OKCANCEL, MB_RETRYCANCEL,
    MB_YESNO, MB_YESNOCANCEL, MSG, PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN, WM_CLOSE, WM_COMMAND,
    WM_QUIT, WINDOW_STYLE, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

/// `CF_UNICODETEXT`.
const UNICODE_TEXT: u32 = 13;
/// `BFFM_INITIALIZED` and `BFFM_SETSELECTIONW` (`WM_USER + 103`).
const BROWSE_INITIALIZED: u32 = 1;
const BROWSE_SET_SELECTION: u32 = 0x0400 + 103;

/// The window a modal dialog belongs to: the thread's active one.
fn owner() -> Option<HWND> {
    // SAFETY: a plain query of the calling thread's state.
    let hwnd = unsafe { GetActiveWindow() };
    (!hwnd.is_invalid()).then_some(hwnd)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A wide string's pointer, or null for an empty one (the system's default).
fn optional(text: &[u16]) -> PCWSTR {
    if text.len() > 1 {
        PCWSTR(text.as_ptr())
    } else {
        PCWSTR::null()
    }
}

/// Tcl paths use forward slashes (`file normalize`'s form), as Tk returns.
fn tcl_path(units: &[u16]) -> String {
    String::from_utf16_lossy(units).replace('\\', "/")
}

/// `tk_messageBox` as the system's message box.
pub fn message_box(request: &MessageBoxRequest) -> String {
    let fallback = rtcl_tk_runtime::default_message_box_button(request);
    let Some(buttons) = rtcl_tk_runtime::message_box_buttons(&request.kind) else {
        return fallback;
    };
    let kind = match request.kind.as_str() {
        "okcancel" => MB_OKCANCEL,
        "yesno" => MB_YESNO,
        "yesnocancel" => MB_YESNOCANCEL,
        "retrycancel" => MB_RETRYCANCEL,
        "abortretryignore" => MB_ABORTRETRYIGNORE,
        _ => MB_OK,
    };
    let icon = match request.icon.as_str() {
        "error" => MB_ICONERROR,
        "warning" => MB_ICONWARNING,
        "question" => MB_ICONQUESTION,
        _ => MB_ICONINFORMATION,
    };
    let default = match buttons.iter().position(|name| *name == fallback) {
        Some(1) => MB_DEFBUTTON2,
        Some(2) => MB_DEFBUTTON3,
        _ => MB_DEFBUTTON1,
    };
    // Tk shows -detail under the message.
    let mut text = request.message.clone();
    if !request.detail.is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&request.detail);
    }
    // SAFETY: valid strings; the owner is a live window of this thread.
    let answer = unsafe {
        MessageBoxW(
            owner(),
            &HSTRING::from(text),
            &HSTRING::from(request.title.as_str()),
            kind | icon | default,
        )
    };
    // IDOK, IDCANCEL, IDABORT, IDRETRY, IDIGNORE, IDYES, IDNO.
    let name = match answer.0 {
        1 => "ok",
        2 => "cancel",
        3 => "abort",
        4 => "retry",
        5 => "ignore",
        6 => "yes",
        7 => "no",
        _ => return fallback,
    };
    name.to_string()
}

/// `tk_getOpenFile`/`tk_getSaveFile`/`tk_chooseDirectory` as the system's
/// dialogs.
pub fn file_dialog(request: &FileDialogRequest) -> Option<Vec<String>> {
    if request.kind == FileDialogKind::Directory {
        return choose_directory(request).map(|directory| vec![directory]);
    }
    // "Description\0*.a;*.b\0" pairs, double-NUL terminated.
    let mut filter: Vec<u16> = Vec::new();
    for (description, patterns) in &request.file_types {
        let globs = patterns
            .iter()
            .map(|pattern| {
                if pattern == "*" {
                    "*.*".to_string()
                } else if pattern.starts_with('.') {
                    format!("*{pattern}")
                } else {
                    pattern.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(";");
        filter.extend(description.encode_utf16());
        filter.push(0);
        filter.extend(globs.encode_utf16());
        filter.push(0);
    }
    filter.push(0);
    // Room for a multiple selection: the directory, then each name.
    let mut file = vec![0u16; if request.multiple { 1 << 15 } else { 1 << 10 }];
    let room = file.len() - 1;
    for (slot, unit) in file
        .iter_mut()
        .take(room)
        .zip(request.initial_file.encode_utf16())
    {
        *slot = unit;
    }
    let initial_dir = wide(&request.initial_dir.replace('/', "\\"));
    let title = wide(&request.title);
    let extension = wide(request.default_extension.trim_start_matches('.'));
    let save = request.kind == FileDialogKind::Save;
    // The dialog must not move the process's working directory.
    let mut flags = OFN_EXPLORER | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST;
    if save {
        if request.confirm_overwrite {
            flags |= OFN_OVERWRITEPROMPT;
        }
    } else {
        flags |= OFN_FILEMUSTEXIST;
        if request.multiple {
            flags |= OFN_ALLOWMULTISELECT;
        }
    }
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner().unwrap_or_default(),
        lpstrFilter: if request.file_types.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(filter.as_ptr())
        },
        nFilterIndex: 1,
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrInitialDir: optional(&initial_dir),
        lpstrTitle: optional(&title),
        Flags: flags,
        lpstrDefExt: optional(&extension),
        ..Default::default()
    };
    // SAFETY: the struct and every buffer it points to outlive the call.
    let answered = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    if !answered.as_bool() {
        return None;
    }
    // One path, or (several files) the directory then each name.
    let parts: Vec<&[u16]> = file
        .split(|unit| *unit == 0)
        .take_while(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [] => None,
        [single] => Some(vec![tcl_path(single)]),
        [directory, names @ ..] => {
            let directory = tcl_path(directory);
            let directory = directory.trim_end_matches('/');
            Some(
                names
                    .iter()
                    .map(|name| format!("{directory}/{}", tcl_path(name)))
                    .collect(),
            )
        }
    }
}

/// Selects the initial directory (handed over as the callback's data) once
/// the folder browser is up.
unsafe extern "system" fn browse_callback(
    hwnd: HWND,
    message: u32,
    _lparam: LPARAM,
    data: LPARAM,
) -> i32 {
    if message == BROWSE_INITIALIZED && data.0 != 0 {
        // SAFETY: `data` is the NUL-terminated path the caller keeps alive
        // for the whole modal call; wParam TRUE says it is a path.
        unsafe { SendMessageW(hwnd, BROWSE_SET_SELECTION, Some(WPARAM(1)), Some(data)) };
    }
    0
}

fn choose_directory(request: &FileDialogRequest) -> Option<String> {
    let title = wide(&request.title);
    let initial = wide(&request.initial_dir.replace('/', "\\"));
    let mut display = [0u16; 260];
    // The new-style browser needs an apartment; a thread already in the
    // multithreaded one gets the classic tree.
    // SAFETY: balanced by the CoUninitialize below.
    let apartment = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let mut flags = BIF_RETURNONLYFSDIRS;
    if apartment {
        flags |= BIF_NEWDIALOGSTYLE;
    }
    let info = BROWSEINFOW {
        hwndOwner: owner().unwrap_or_default(),
        pszDisplayName: PWSTR(display.as_mut_ptr()),
        lpszTitle: optional(&title),
        ulFlags: flags,
        lpfn: Some(browse_callback),
        lParam: LPARAM(if initial.len() > 1 {
            initial.as_ptr() as isize
        } else {
            0
        }),
        ..Default::default()
    };
    // SAFETY: `info` and its buffers outlive the modal call; the item list
    // the shell returns is freed with its allocator.
    let chosen = unsafe {
        let list = SHBrowseForFolderW(&info);
        let mut path = [0u16; 260];
        let chosen = (!list.is_null() && SHGetPathFromIDListW(list, &mut path).as_bool())
            .then(|| {
                let length = path.iter().position(|unit| *unit == 0).unwrap_or(path.len());
                tcl_path(&path[..length])
            });
        if !list.is_null() {
            CoTaskMemFree(Some(list as *const _));
        }
        chosen
    };
    if apartment {
        // SAFETY: pairs with the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
    chosen
}

thread_local! {
    /// The color dialog's sixteen custom colors, kept for the session.
    static CUSTOM_COLORS: std::cell::RefCell<[COLORREF; 16]> =
        const { std::cell::RefCell::new([COLORREF(0x00ff_ffff); 16]) };
}

/// `tk_chooseColor` as the system's color dialog.
pub fn color_dialog(request: &ColorDialogRequest) -> Option<String> {
    // A COLORREF is 0x00bbggrr.
    let initial = super::parse_color(&request.initial).map(|color| {
        let rgb = color.light.to_rgb();
        let byte = |channel: f32| (channel * 255.0).round().clamp(0.0, 255.0) as u32;
        COLORREF(byte(rgb.r) | (byte(rgb.g) << 8) | (byte(rgb.b) << 16))
    });
    let mut custom = CUSTOM_COLORS.with(|colors| *colors.borrow());
    let mut dialog = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner().unwrap_or_default(),
        rgbResult: initial.unwrap_or(COLORREF(0)),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_FULLOPEN
            | if initial.is_some() {
                CC_RGBINIT
            } else {
                CHOOSECOLOR_FLAGS(0)
            },
        ..Default::default()
    };
    // SAFETY: the struct and the custom colors outlive the modal call.
    let answered = unsafe { ChooseColorW(&mut dialog) }.as_bool();
    CUSTOM_COLORS.with(|colors| *colors.borrow_mut() = custom);
    if !answered {
        return None;
    }
    let rgb = dialog.rgbResult.0;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        rgb & 0xff,
        (rgb >> 8) & 0xff,
        (rgb >> 16) & 0xff
    ))
}

/// The system clipboard's text, for `clipboard get`.
pub fn clipboard_text() -> String {
    // SAFETY: the clipboard is opened and closed on this thread, and the
    // data handle is locked only while its NUL-terminated text is read.
    unsafe {
        if OpenClipboard(owner()).is_err() {
            return String::new();
        }
        let mut text = String::new();
        if let Ok(handle) = GetClipboardData(UNICODE_TEXT) {
            let global = HGLOBAL(handle.0);
            let units = GlobalLock(global) as *const u16;
            if !units.is_null() {
                let mut length = 0;
                while *units.add(length) != 0 {
                    length += 1;
                }
                text = String::from_utf16_lossy(std::slice::from_raw_parts(units, length));
                let _ = GlobalUnlock(global);
            }
        }
        let _ = CloseClipboard();
        text
    }
}

/// The primary monitor's size in pixels (`winfo screenwidth/height`).
pub fn screen_metrics() -> (i64, i64) {
    // SAFETY: plain queries.
    let (width, height) =
        unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    if width > 0 && height > 0 {
        (i64::from(width), i64::from(height))
    } else {
        (1920, 1080)
    }
}

/// `bell`: the system's default sound.
pub fn ring_bell() {
    // SAFETY: a plain call.
    let _ = unsafe { MessageBeep(MB_OK) };
}

/// One non-blocking pass over the thread's message queue: delivers clicks,
/// menus and input to the re-entrant evals a `vwait` nested loop waits for.
pub fn pump_native_events() -> bool {
    let mut pumped = false;
    let mut message = MSG::default();
    for _ in 0..64 {
        // SAFETY: the message is this thread's own; a WM_QUIT is handed
        // back to the app's loop, which is the one to end on it.
        unsafe {
            if !PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                break;
            }
            if message.message == WM_QUIT {
                PostQuitMessage(message.wParam.0 as i32);
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        pumped = true;
    }
    pumped
}

/// Button commands start at this dialog-item id; the index is the offset.
const BUTTON_ID_BASE: i32 = 100;

/// The accumulated words of an in-memory dialog template.
struct DialogTemplate {
    words: Vec<u16>,
}

impl DialogTemplate {
    fn new(style: WINDOW_STYLE, items: u16, title: &str, rect: [i16; 4]) -> Self {
        let mut words = Vec::new();
        let mut push32 = |words: &mut Vec<u16>, value: u32| {
            words.push(value as u16);
            words.push((value >> 16) as u16);
        };
        push32(&mut words, style.0);
        push32(&mut words, 0); // extended style
        words.push(items);
        words.extend(rect.map(|v| v as u16));
        words.push(0); // no menu
        words.push(0); // no window class
        words.extend(title.encode_utf16());
        words.push(0);
        // A font block (DS_SETFONT on the classic template is just the
        // point size followed by the typeface): 9 pt Segoe UI.
        words.push(9);
        words.extend("Segoe UI".encode_utf16());
        words.push(0);
        Self { words }
    }

    fn item(&mut self, style: WINDOW_STYLE, rect: [i16; 4], id: i32, class: u16, label: &str) {
        // Items align on a DWORD boundary.
        if self.words.len() % 2 == 1 {
            self.words.push(0);
        }
        let mut push32 = |words: &mut Vec<u16>, value: u32| {
            words.push(value as u16);
            words.push((value >> 16) as u16);
        };
        push32(&mut self.words, style.0); // style
        push32(&mut self.words, 0); // extended style
        self.words.extend(rect.map(|v| v as u16));
        self.words.push(id as u16);
        self.words.push(0xFFFF);
        self.words.push(class); // button = 0x0080, static = 0x0082
        self.words.extend(label.encode_utf16());
        self.words.push(0);
        self.words.push(0); // no extra data
    }

    fn run(self, owner: Option<HWND>) -> i32 {
        let proc: DLGPROC = Some(dialog_proc);
        // SAFETY: the template outlives the modal call; the proc and the
        // owner window are valid for it.
        unsafe {
            DialogBoxIndirectParamW(
                None,
                self.words.as_ptr() as *const DLGTEMPLATE,
                owner,
                proc,
                LPARAM(0),
            ) as i32
        }
    }
}

/// Ends the dialog on a button command or a close; Enter/Escape map to the
/// default (the caller translates the id).
unsafe extern "system" fn dialog_proc(hwnd: HWND, message: u32, w: WPARAM, _l: LPARAM) -> isize {
    match message {
        WM_COMMAND => {
            let id = w.0 as i32 & 0xffff;
            // SAFETY: hwnd is this live dialog.
            unsafe { EndDialog(hwnd, id as isize) };
            1
        }
        WM_CLOSE => {
            // SAFETY: as above; IDCANCEL.
            unsafe { EndDialog(hwnd, 2) };
            1
        }
        _ => 0,
    }
}

/// `tk_dialog` as a classic modal dialog box: the message on top, the app's
/// buttons along the bottom (a memory template, so no comctl32 v6 is
/// needed for `TaskDialogIndirect`).
pub fn tk_dialog(request: &rtcl_tk_runtime::DialogRequest) -> Option<usize> {
    if request.buttons.is_empty() {
        return None;
    }
    let mut message = request.message.clone();
    if !request.detail.is_empty() {
        if !message.is_empty() {
            message.push_str("\n\n");
        }
        message.push_str(&request.detail);
    }
    if message.is_empty() {
        message = " ".to_string();
    }
    let lines = message.split('\n').count().max(1) as i32;
    let longest = message.split('\n').map(str::chars).map(Iterator::count).max().unwrap_or(1) as i32;
    // Dialog units: ~4 px wide, ~8 px tall at 96 dpi.
    let width = ((longest * 2) + 20).clamp(110, 220);
    let text_height = lines * 8 + 4;
    let button_y = text_height + 10;
    let count = request.buttons.len() as i32;
    let button_width = 52;
    let gap = 8;
    let row = count * button_width + (count - 1) * gap;
    let start = (width - row) / 2;

    let title = if request.title.is_empty() {
        " ".to_string()
    } else {
        request.title.clone()
    };
    let mut template = DialogTemplate::new(
        WINDOW_STYLE(
            (DS_MODALFRAME | DS_CENTER | DS_SETFONT) as u32
                | WS_POPUP.0
                | WS_CAPTION.0
                | WS_SYSMENU.0,
        ),
        (count + 1) as u16,
        &title,
        [0, 0, width as i16, (button_y + 22) as i16],
    );
    template.item(
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0), // SS_LEFT = 0
        [10, 6, (width - 20) as i16, text_height as i16],
        0,
        0x0082,
        &message,
    );
    for (index, label) in request.buttons.iter().enumerate() {
        // BS_* are button styles (i32): BS_PUSHBUTTON = 0, DEFBUTTON = 1.
        let style = if index == request.default {
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | 0x0000_0001)
        } else {
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0)
        };
        template.item(
            style,
            [
                (start + index as i32 * (button_width + gap)) as i16,
                button_y as i16,
                button_width as i16,
                14,
            ],
            BUTTON_ID_BASE + index as i32,
            0x0080,
            label,
        );
    }
    let answer = template.run(owner());
    match answer {
        1 | 2 => Some(request.default), // Enter/Escape with no matching button
        id if id >= BUTTON_ID_BASE => Some((id - BUTTON_ID_BASE) as usize),
        _ => Some(request.default),
    }
}
