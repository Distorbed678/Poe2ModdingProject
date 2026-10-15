//! Window messages → imgui input, plus the overlay hotkeys.

use std::sync::atomic::Ordering;

use imgui::{Io, Key, MouseButton};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    UI::{
        Input::{GetRawInputData, HRAWINPUT, RAWINPUTHEADER, RID_HEADER, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE},
        WindowsAndMessaging::{
            DefWindowProcW, GetClientRect, WM_CHAR, WM_INPUT, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDBLCLK,
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
            WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

use super::{EXTENT, INPUT, WANT_KEYBOARD, WANT_MOUSE};

pub enum Event {
    Mouse([f32; 2]),
    Button(MouseButton, bool),
    Wheel(f32),
    Key(Key, bool),
    Char(char),
}

impl Event {
    pub fn apply(self, io: &mut Io) {
        match self {
            Event::Mouse(p) => io.add_mouse_pos_event(p),
            Event::Button(b, down) => io.add_mouse_button_event(b, down),
            Event::Wheel(v) => io.add_mouse_wheel_event([0.0, v]),
            Event::Key(k, down) => io.add_key_event(k, down),
            Event::Char(c) => io.add_input_character(c),
        }
    }
}

fn push(event: Event) {
    INPUT.lock().unwrap().push(event);
}

/// Client coordinates → swapchain pixels.
fn mouse_pos(hwnd: isize, lparam: isize) -> [f32; 2] {
    let x = (lparam & 0xffff) as i16 as f32;
    let y = ((lparam >> 16) & 0xffff) as i16 as f32;
    let mut r: RECT = unsafe { std::mem::zeroed() };
    let (w, h) = (EXTENT.0.load(Ordering::Relaxed), EXTENT.1.load(Ordering::Relaxed));
    if unsafe { GetClientRect(hwnd as HWND, &mut r) } != 0 && r.right > 0 && r.bottom > 0 && w > 0 {
        [x * w as f32 / r.right as f32, y * h as f32 / r.bottom as f32]
    } else {
        [x, y]
    }
}

pub fn window_message(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> Option<isize> {
    let cfg = &crate::config().overlay;
    let key_down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
    let key_up = matches!(msg, WM_KEYUP | WM_SYSKEYUP);
    // Hotkeys (first press only, not auto-repeat).
    if key_down || key_up {
        let repeat = lparam & (1 << 30) != 0;
        if wparam == cfg.toggle_key as usize {
            if key_down && !repeat {
                let v = !super::visible();
                super::set_visible(v);
                log::debug!("overlay {}", if v { "shown" } else { "hidden" });
            }
            return Some(0);
        }
        if wparam == cfg.snapshot_key as usize {
            if key_down && !repeat {
                crate::snapshot::request();
            }
            return Some(0);
        }
    }
    if !super::visible() {
        return None;
    }
    let want_mouse = WANT_MOUSE.load(Ordering::Relaxed);
    let want_keyboard = WANT_KEYBOARD.load(Ordering::Relaxed);
    let consume = |yes: bool| yes.then_some(0);
    match msg {
        WM_MOUSEMOVE => {
            push(Event::Mouse(mouse_pos(hwnd, lparam)));
            None
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK
        | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
            let (button, down) = match msg {
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => (MouseButton::Left, true),
                WM_LBUTTONUP => (MouseButton::Left, false),
                WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => (MouseButton::Right, true),
                WM_RBUTTONUP => (MouseButton::Right, false),
                WM_MBUTTONDOWN => (MouseButton::Middle, true),
                _ => (MouseButton::Middle, false),
            };
            push(Event::Mouse(mouse_pos(hwnd, lparam)));
            push(Event::Button(button, down));
            // Releases always reach the game, so it never sees a button stuck down.
            consume(want_mouse && down)
        }
        WM_MOUSEWHEEL => {
            push(Event::Wheel(((wparam >> 16) as i16) as f32 / 120.0));
            consume(want_mouse)
        }
        _ if key_down || key_up => {
            if let Some(k) = map_key(wparam) {
                push(Event::Key(k, key_down));
            }
            for (vk, m) in [(0x11, Key::ModCtrl), (0x10, Key::ModShift), (0x12, Key::ModAlt)] {
                if wparam == vk {
                    push(Event::Key(m, key_down));
                }
            }
            consume(want_keyboard && key_down)
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam as u32).filter(|c| !c.is_control()) {
                push(Event::Char(c));
            }
            consume(want_keyboard)
        }
        WM_INPUT => {
            // The game reads movement and skills from Raw Input: hide those while imgui has
            // the mouse or keyboard. DefWindowProc must still see the message.
            let mut header: RAWINPUTHEADER = unsafe { std::mem::zeroed() };
            let mut size = size_of::<RAWINPUTHEADER>() as u32;
            let n = unsafe {
                GetRawInputData(
                    lparam as HRAWINPUT,
                    RID_HEADER,
                    &mut header as *mut _ as *mut _,
                    &mut size,
                    size_of::<RAWINPUTHEADER>() as u32,
                )
            };
            let hide = n != u32::MAX
                && ((header.dwType == RIM_TYPEMOUSE && want_mouse)
                    || (header.dwType == RIM_TYPEKEYBOARD && want_keyboard));
            hide.then(|| unsafe { DefWindowProcW(hwnd as HWND, msg, wparam, lparam) })
        }
        _ => None,
    }
}

fn map_key(vk: usize) -> Option<Key> {
    Some(match vk {
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x0d => Key::Enter,
        0x1b => Key::Escape,
        0x20 => Key::Space,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::LeftArrow,
        0x26 => Key::UpArrow,
        0x27 => Key::RightArrow,
        0x28 => Key::DownArrow,
        0x2d => Key::Insert,
        0x2e => Key::Delete,
        0x10 | 0xa0 => Key::LeftShift,
        0xa1 => Key::RightShift,
        0x11 | 0xa2 => Key::LeftCtrl,
        0xa3 => Key::RightCtrl,
        0x12 | 0xa4 => Key::LeftAlt,
        0xa5 => Key::RightAlt,
        0x30..=0x39 => [
            Key::Key0,
            Key::Key1,
            Key::Key2,
            Key::Key3,
            Key::Key4,
            Key::Key5,
            Key::Key6,
            Key::Key7,
            Key::Key8,
            Key::Key9,
        ][vk - 0x30],
        0x41..=0x5a => [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
        ][vk - 0x41],
        _ => return None,
    })
}
