//! Global hold-to-talk hotkey via a low-level keyboard hook (WH_KEYBOARD_LL).
//!
//! A hotkey is a combo of one or more key-classes, parsed from a string:
//!   "rctrl"             single key
//!   "f8"                single key
//!   "ctrl+shift+space"  chord (all must be held)
//! A class like "ctrl" matches either physical Ctrl. The chord is *active* while
//! every class has at least one of its keys down; releasing any class ends it.
//! Hold-to-talk fires `Down` when the chord becomes active and `Up` when it ends.

use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HMODULE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Up,
}

struct HookState {
    classes: Vec<Vec<u32>>, // each class = acceptable VK codes
    relevant: HashSet<u32>,
    pressed: HashSet<u32>,
    active: bool,
    tx: Sender<HotkeyEvent>,
}

/// Hook state lives behind `Option` so the hook can be torn down and reinstalled
/// (the engine restarts in-process on a model/engine change — no app relaunch).
/// `OnceLock` only guards one-time creation of the mutex; the inner `Option` is
/// swapped each `run`.
static STATE: OnceLock<Mutex<Option<HookState>>> = OnceLock::new();

fn state() -> &'static Mutex<Option<HookState>> {
    STATE.get_or_init(|| Mutex::new(None))
}

/// Map a token to its set of virtual-key codes. Returns None for unknown tokens.
fn token_vks(tok: &str) -> Option<Vec<u32>> {
    let t = tok.trim().to_lowercase();
    let v = match t.as_str() {
        "ctrl" | "control" => vec![0xA2, 0xA3],
        "shift" => vec![0xA0, 0xA1],
        "alt" => vec![0xA4, 0xA5],
        "win" | "super" | "meta" => vec![0x5B, 0x5C],
        "lctrl" => vec![0xA2],
        "rctrl" => vec![0xA3],
        "lshift" => vec![0xA0],
        "rshift" => vec![0xA1],
        "lalt" => vec![0xA4],
        "ralt" => vec![0xA5],
        "space" => vec![0x20],
        "tab" => vec![0x09],
        "enter" | "return" => vec![0x0D],
        "esc" | "escape" => vec![0x1B],
        "capslock" | "caps" => vec![0x14],
        _ => {
            if let Some(n) = t.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
                if (1..=24).contains(&n) {
                    return Some(vec![0x70 + (n - 1)]); // VK_F1 = 0x70
                }
                return None;
            }
            if t.len() == 1 {
                let c = t.chars().next().unwrap();
                if c.is_ascii_lowercase() {
                    return Some(vec![0x41 + (c as u32 - 'a' as u32)]);
                }
                if c.is_ascii_digit() {
                    return Some(vec![0x30 + (c as u32 - '0' as u32)]);
                }
            }
            return None;
        }
    };
    Some(v)
}

/// Parse a combo string into key-classes. Errors on any unknown token.
pub fn parse_spec(s: &str) -> Result<Vec<Vec<u32>>, String> {
    let classes: Result<Vec<_>, _> = s
        .split('+')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| token_vks(t).ok_or_else(|| format!("unknown hotkey token '{t}'")))
        .collect();
    let classes = classes?;
    if classes.is_empty() {
        return Err("empty hotkey".into());
    }
    Ok(classes)
}

/// The chord is active when every class has at least one of its keys pressed.
fn chord_active(classes: &[Vec<u32>], pressed: &HashSet<u32>) -> bool {
    classes
        .iter()
        .all(|cls| cls.iter().any(|vk| pressed.contains(vk)))
}

fn is_active(st: &HookState) -> bool {
    chord_active(&st.classes, &st.pressed)
}

unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;
        if !injected {
            let mut guard = state().lock().unwrap();
            if let Some(st) = guard.as_mut() {
                if st.relevant.contains(&kb.vkCode) {
                    match wparam.0 as u32 {
                        WM_KEYDOWN | WM_SYSKEYDOWN => {
                            st.pressed.insert(kb.vkCode);
                        }
                        WM_KEYUP | WM_SYSKEYUP => {
                            st.pressed.remove(&kb.vkCode);
                        }
                        _ => {}
                    }
                    let now = is_active(st);
                    if now != st.active {
                        st.active = now;
                        let _ = st.tx.send(if now { HotkeyEvent::Down } else { HotkeyEvent::Up });
                    }
                }
            }
        }
    }
    CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam)
}

/// Install the hook and pump messages. BLOCKS — run on a dedicated thread.
///
/// `ready` is called once with this thread's Win32 id after the message queue
/// exists, so the caller can later post `WM_QUIT` (via [`stop_thread`]) to tear
/// the hook down for an in-process engine restart without losing the message.
pub fn run(spec: &str, tx: Sender<HotkeyEvent>, ready: impl FnOnce(u32)) -> Result<(), String> {
    let classes = parse_spec(spec)?;
    let relevant: HashSet<u32> = classes.iter().flatten().copied().collect();
    *state().lock().unwrap() = Some(HookState {
        classes,
        relevant,
        pressed: HashSet::new(),
        active: false,
        tx,
    });

    unsafe {
        let hmod: HMODULE = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), hmod, 0)
            .map_err(|e| format!("SetWindowsHookExW: {e}"))?;
        // Force this thread's message queue into existence so a WM_QUIT posted by
        // stop_thread can't be dropped, then hand our thread id to the caller.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        ready(GetCurrentThreadId());
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
    // Clear state so a fresh `run` can reinstall cleanly.
    *state().lock().unwrap() = None;
    Ok(())
}

/// Post `WM_QUIT` to a hotkey thread (by the Win32 id reported to `run`'s `ready`
/// callback) so its message loop exits and the hook is uninstalled. No-op for 0.
pub fn stop_thread(tid: u32) {
    if tid == 0 {
        return;
    }
    unsafe {
        let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_and_chord() {
        assert_eq!(parse_spec("rctrl").unwrap(), vec![vec![0xA3]]);
        assert_eq!(parse_spec("f8").unwrap(), vec![vec![0x77]]);
        assert_eq!(
            parse_spec("ctrl+shift+space").unwrap(),
            vec![vec![0xA2, 0xA3], vec![0xA0, 0xA1], vec![0x20]]
        );
        assert_eq!(parse_spec("ctrl+a").unwrap(), vec![vec![0xA2, 0xA3], vec![0x41]]);
        assert!(parse_spec("frobnicate").is_err());
        assert!(parse_spec("").is_err());
    }

    #[test]
    fn chord_activation() {
        let classes = parse_spec("ctrl+shift+space").unwrap();
        let mut p = HashSet::new();
        assert!(!chord_active(&classes, &p));
        p.insert(0xA2); // left ctrl
        p.insert(0xA1); // right shift
        assert!(!chord_active(&classes, &p)); // space missing
        p.insert(0x20); // space
        assert!(chord_active(&classes, &p)); // all held
        p.remove(&0x20);
        assert!(!chord_active(&classes, &p)); // releasing space ends it
    }
}
