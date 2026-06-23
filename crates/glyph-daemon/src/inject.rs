//! Text injection at the cursor + focused-app awareness (Phase 0 verified).
//! Primary = clipboard paste (save -> set -> Ctrl+V -> restore). Fallback =
//! per-char Unicode SendInput for apps that block paste.

use std::thread;
use std::time::Duration;

use windows::core::PWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Paste,
    Unicode,
}

fn key_event(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn chord_ctrl(vk_char: u16) {
    let inputs = [
        key_event(VK_CONTROL, 0, KEYBD_EVENT_FLAGS(0)),
        key_event(VIRTUAL_KEY(vk_char), 0, KEYBD_EVENT_FLAGS(0)),
        key_event(VIRTUAL_KEY(vk_char), 0, KEYEVENTF_KEYUP),
        key_event(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];
    send(&inputs);
}

fn inject_unicode(text: &str) {
    for u in text.encode_utf16() {
        let pair = [
            key_event(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE),
            key_event(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        ];
        send(&pair);
        thread::sleep(Duration::from_millis(2));
    }
}

/// Put `text` on the clipboard. If `keep` is false, returns the previous text so
/// the caller can restore it.
fn set_clipboard(text: &str, keep: bool) -> Option<String> {
    let mut cb = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[inject] clipboard open failed: {e}");
            return None;
        }
    };
    let saved = if keep { None } else { cb.get_text().ok() };
    if cb.set_text(text.to_string()).is_err() {
        eprintln!("[inject] clipboard set failed");
    }
    saved
}

/// Inject `text` at the cursor. The dictated text is always placed on the
/// clipboard; with `keep_on_clipboard` it stays there (always pasteable),
/// otherwise the previous clipboard contents are restored after pasting.
pub fn inject(text: &str, method: Method, keep_on_clipboard: bool) {
    if text.is_empty() {
        return;
    }
    // Always put the text on the clipboard (it's the paste source, and a reliable
    // fallback even for the unicode method).
    let saved = set_clipboard(text, keep_on_clipboard);

    match method {
        Method::Paste => {
            thread::sleep(Duration::from_millis(30));
            chord_ctrl(0x56); // Ctrl+V
            thread::sleep(Duration::from_millis(150));
        }
        Method::Unicode => inject_unicode(text),
    }

    if let Some(prev) = saved {
        // Only when not keeping: restore the user's previous clipboard.
        let mut cb = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(_) => return,
        };
        let _ = cb.set_text(prev);
    }
}

/// (process_name, window_title) of the focused window.
pub fn foreground_app() -> (String, String) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut title = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut title);
        let title = String::from_utf16_lossy(&title[..n as usize]);

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut proc_name = String::from("<unknown>");
        if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_FORMAT(0), PWSTR(buf.as_mut_ptr()), &mut len)
                .is_ok()
            {
                let full = String::from_utf16_lossy(&buf[..len as usize]);
                proc_name = full.rsplit('\\').next().unwrap_or(&full).to_string();
            }
            let _ = CloseHandle(h);
        }
        (proc_name, title)
    }
}
