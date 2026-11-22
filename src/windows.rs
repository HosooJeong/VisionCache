//! Native Win32 capture and input. All GDI handles stay on the calling thread.

use std::{collections::HashSet, mem::size_of};

use anyhow::{Context, Result, bail, ensure};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
        HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
    },
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
            KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
            MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
        },
        WindowsAndMessaging::{
            EnumWindows, GA_ROOT, GW_OWNER, GetAncestor, GetClientRect, GetForegroundWindow,
            GetWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
            IsWindow, IsWindowVisible, SetCursorPos, WindowFromPoint,
        },
    },
};

use crate::{
    desktop::{Desktop, Input, ScreenBounds, ScreenPoint, Window, WindowState},
    image::Raster,
};

pub struct WindowsDesktop;

struct DpiScope(DPI_AWARENESS_CONTEXT);

impl DpiScope {
    fn enter() -> Result<Self> {
        // Set and restore on the same thread so all geometry uses physical pixels.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        ensure!(
            !previous.is_null(),
            "Could not establish per-monitor Windows DPI coordinates."
        );
        Ok(Self(previous))
    }
}

impl Drop for DpiScope {
    fn drop(&mut self) {
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

struct ClientDc(HWND, HDC);
impl Drop for ClientDc {
    fn drop(&mut self) {
        unsafe { ReleaseDC(self.0, self.1) };
    }
}

struct MemoryDc(HDC);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        unsafe { DeleteDC(self.0) };
    }
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

struct SelectedBitmap(HDC, HGDIOBJ);
impl Drop for SelectedBitmap {
    fn drop(&mut self) {
        unsafe { SelectObject(self.0, self.1) };
    }
}

fn handle(value: u64) -> Result<HWND> {
    let address = usize::try_from(value).context("Invalid Windows handle.")?;
    ensure!(address != 0, "Invalid Windows handle.");
    Ok(address as HWND)
}

fn identify(hwnd: HWND) -> Result<Window> {
    // HWND values may become stale at any time; validate them before each use.
    ensure!(
        unsafe { IsWindow(hwnd) != 0 && GetAncestor(hwnd, GA_ROOT) == hwnd },
        "The selected top-level window is no longer available."
    );
    ensure!(
        unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 },
        "Restore the selected window before capturing it."
    );
    let mut pid = 0;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    ensure!(tid != 0, "The selected window closed.");
    let capacity = unsafe { GetWindowTextLengthW(hwnd) }
        .max(0)
        .saturating_add(1);
    let mut title = vec![0u16; capacity as usize];
    let read = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), capacity) }.max(0) as usize;
    Ok(Window {
        handle: hwnd as usize as u64,
        process_id: pid,
        thread_id: Some(tid),
        title: String::from_utf16_lossy(&title[..read]),
    })
}

fn state(hwnd: HWND) -> Result<WindowState> {
    let window = identify(hwnd)?;
    let mut rect = RECT::default();
    ensure!(
        unsafe { GetClientRect(hwnd, &mut rect) } != 0,
        "Could not read the client rectangle."
    );
    let mut top_left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let mut bottom_right = POINT {
        x: rect.right,
        y: rect.bottom,
    };
    ensure!(
        unsafe {
            ClientToScreen(hwnd, &mut top_left) != 0 && ClientToScreen(hwnd, &mut bottom_right) != 0
        },
        "Could not locate the selected window's client area."
    );
    ensure!(
        bottom_right.x > top_left.x && bottom_right.y > top_left.y,
        "The selected window has an empty client area."
    );
    Ok(WindowState {
        window,
        bounds: ScreenBounds::Pixels([top_left.x, top_left.y, bottom_right.x, bottom_right.y]),
        attached_sheet: false,
        raster_size: None,
    })
}

fn capture_target(root: HWND, follow_owned: bool) -> Result<HWND> {
    if !follow_owned {
        return Ok(root);
    }
    let base = identify(root)?;
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() || foreground == root {
        return Ok(root);
    }
    let Ok(target) = identify(foreground) else {
        return Ok(root);
    };
    if target.process_id != base.process_id {
        return Ok(root);
    }
    let mut node = foreground;
    let mut seen = HashSet::new();
    while !node.is_null() && seen.insert(node) {
        if node == root {
            return Ok(foreground);
        }
        node = unsafe { GetWindow(node, GW_OWNER) };
    }
    Ok(root)
}

fn capture_bgr(hwnd: HWND, width: i32, height: i32, frame: &mut Raster) -> Result<()> {
    let bytes = frame.prepare(u32::try_from(width)?, u32::try_from(height)?)?;
    let source = unsafe { GetDC(hwnd) };
    ensure!(
        !source.is_null(),
        "Windows could not open the selected window's device context."
    );
    let source = ClientDc(hwnd, source);
    let destination = unsafe { CreateCompatibleDC(source.1) };
    ensure!(
        !destination.is_null(),
        "Windows could not create a capture device context."
    );
    let destination = MemoryDc(destination);
    let bitmap = unsafe { CreateCompatibleBitmap(source.1, width, height) };
    ensure!(
        !bitmap.is_null(),
        "Windows could not allocate a capture bitmap."
    );
    let bitmap = Bitmap(bitmap);
    {
        let old = unsafe { SelectObject(destination.0, bitmap.0) };
        ensure!(
            !old.is_null() && old as isize != -1,
            "Windows could not select the capture bitmap."
        );
        let _selection = SelectedBitmap(destination.0, old);
        ensure!(
            unsafe { BitBlt(destination.0, 0, 0, width, height, source.1, 0, 0, SRCCOPY) } != 0,
            "Windows could not copy the selected window."
        );
    }

    let mut description = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // Top-down BGR24.
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    // Bitmap is deselected. With 24 bits per pixel, GetDIBits writes padded scanlines
    // into the Rust buffer prepared above.
    let copied = unsafe {
        GetDIBits(
            destination.0,
            bitmap.0,
            0,
            height as u32,
            bytes.as_mut_ptr().cast(),
            &mut description,
            DIB_RGB_COLORS,
        )
    };
    ensure!(copied == height, "Windows returned an incomplete capture.");
    Ok(())
}

fn require_foreground(snapshot: &WindowState) -> Result<HWND> {
    let hwnd = handle(snapshot.window.handle)?;
    ensure!(
        state(hwnd)?.same_placement(snapshot),
        "The window placement changed. Take another observation."
    );
    ensure!(
        unsafe { GetForegroundWindow() } == hwnd,
        "Focus the observed window and capture it again before sending input."
    );
    Ok(hwnd)
}

fn position(snapshot: &WindowState, point: ScreenPoint) -> Result<()> {
    let hwnd = require_foreground(snapshot)?;
    let (x, y) = point.pixels()?;
    ensure!(
        snapshot.bounds.contains(point),
        "The point is outside the captured client area."
    );
    let target_at_point = || {
        let hit = unsafe { WindowFromPoint(POINT { x, y }) };
        !hit.is_null() && unsafe { GetAncestor(hit, GA_ROOT) } == hwnd
    };
    ensure!(
        target_at_point(),
        "The point is not visible in the target window."
    );
    ensure!(
        unsafe { SetCursorPos(x, y) } != 0,
        "Windows could not move the mouse to the target."
    );
    require_foreground(snapshot)?;
    ensure!(
        target_at_point(),
        "The point is no longer visible in the target window."
    );
    Ok(())
}

fn mouse(flags: u32, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                mouseData: data,
                ..Default::default()
            },
        },
    }
}

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn click_events() -> [INPUT; 2] {
    [mouse(MOUSEEVENTF_LEFTDOWN, 0), mouse(MOUSEEVENTF_LEFTUP, 0)]
}

fn text_events(text: &str) -> Result<Vec<INPUT>> {
    ensure!(
        !text.is_empty()
            && text.chars().take(2001).count() <= 2000
            && !text.chars().any(char::is_control),
        "Provide 1–2000 characters of plain text. Use press_key for control keys."
    );
    Ok(text
        .encode_utf16()
        .flat_map(|unit| {
            [
                key(0, unit, KEYEVENTF_UNICODE),
                key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
            ]
        })
        .collect())
}

fn chord_events(chord: &str) -> Result<Vec<INPUT>> {
    let chord = chord.trim().to_ascii_lowercase();
    let mut pieces: Vec<_> = chord.split('+').map(str::trim).collect();
    let button = pieces.pop().unwrap_or("");
    let mut modifiers = Vec::with_capacity(3);
    for piece in pieces {
        let code = match piece {
            "ctrl" => 0x11,
            "shift" => 0x10,
            "alt" => 0x12,
            _ => bail!("Unsupported modifier: {piece:?}"),
        };
        ensure!(!modifiers.contains(&code), "Modifiers must not repeat.");
        modifiers.push(code);
    }
    let (code, extended) = match button {
        "backspace" => (0x08, false),
        "tab" => (0x09, false),
        "enter" => (0x0D, false),
        "escape" => (0x1B, false),
        "space" => (0x20, false),
        "pageup" => (0x21, true),
        "pagedown" => (0x22, true),
        "end" => (0x23, true),
        "home" => (0x24, true),
        "left" => (0x25, true),
        "up" => (0x26, true),
        "right" => (0x27, true),
        "down" => (0x28, true),
        "delete" => (0x2E, true),
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => {
            (u16::from(s.as_bytes()[0].to_ascii_uppercase()), false)
        }
        s => {
            let number = s
                .strip_prefix('f')
                .and_then(|n| n.parse::<u16>().ok())
                .filter(|n| (1..=12).contains(n))
                .with_context(|| format!("Unsupported key: {s:?}"))?;
            (0x6f + number, false)
        }
    };
    let flags = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    let mut events = Vec::with_capacity(modifiers.len() * 2 + 2);
    events.extend(modifiers.iter().map(|&modifier| key(modifier, 0, 0)));
    events.extend([key(code, 0, flags), key(code, 0, flags | KEYEVENTF_KEYUP)]);
    events.extend(
        modifiers
            .iter()
            .rev()
            .map(|&modifier| key(modifier, 0, KEYEVENTF_KEYUP)),
    );
    Ok(events)
}

fn send(events: &[INPUT]) -> Result<()> {
    let count = u32::try_from(events.len())?;
    let sent = unsafe { SendInput(count, events.as_ptr(), size_of::<INPUT>() as i32) };
    ensure!(
        sent == count,
        "Windows accepted {sent}/{count} events. Observe before retrying."
    );
    Ok(())
}

impl Desktop for WindowsDesktop {
    fn windows(&self) -> Result<Vec<Window>> {
        unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> i32 {
            if let Ok(window) = identify(hwnd)
                && !window.title.is_empty()
            {
                // EnumWindows invokes this synchronously with the vector pointer supplied below.
                unsafe { (*(param as *mut Vec<Window>)).push(window) };
            }
            1
        }

        let mut windows = Vec::new();
        ensure!(
            unsafe { EnumWindows(Some(visit), (&mut windows as *mut Vec<Window>) as LPARAM) } != 0,
            "Could not enumerate windows in the interactive desktop."
        );
        Ok(windows)
    }

    fn capture(
        &mut self,
        requested: u64,
        follow_owned: bool,
        frame: &mut Raster,
    ) -> Result<WindowState> {
        let _dpi = DpiScope::enter()?;
        let root = handle(requested)?;
        let actual = capture_target(root, follow_owned)?;
        let snapshot = state(actual)?;
        let ScreenBounds::Pixels([left, top, right, bottom]) = snapshot.bounds else {
            bail!("Unexpected Windows coordinate space.");
        };
        capture_bgr(
            actual,
            right.checked_sub(left).context("Window is too wide.")?,
            bottom.checked_sub(top).context("Window is too tall.")?,
            frame,
        )?;
        ensure!(
            capture_target(root, follow_owned)? == actual,
            "The active window changed during capture. Capture it again."
        );
        ensure!(
            state(actual)?.same_placement(&snapshot),
            "The window moved or resized during capture. Capture it again."
        );
        ensure!(
            !frame.all_black(),
            "Windows returned a blank capture for this app."
        );
        Ok(snapshot)
    }

    fn input(&mut self, snapshot: &WindowState, input: Input<'_>) -> Result<()> {
        let _dpi = DpiScope::enter()?;
        match input {
            Input::Click { point } => {
                position(snapshot, point)?;
                send(&click_events())
            }
            Input::Scroll { point, ticks } => {
                ensure!(
                    (-30..=30).contains(&ticks) && ticks != 0,
                    "Use a nonzero scroll count between -30 and 30."
                );
                position(snapshot, point)?;
                send(&[mouse(MOUSEEVENTF_WHEEL, (ticks * 120) as u32)])
            }
            Input::Fill {
                point,
                text,
                replace,
            } => {
                let characters = text_events(text)?;
                let mut events = click_events().to_vec();
                if replace {
                    events.extend(chord_events("ctrl+a")?);
                }
                events.extend(characters);
                position(snapshot, point)?;
                send(&events)
            }
            Input::Text(text) => {
                let events = text_events(text)?;
                require_foreground(snapshot)?;
                send(&events)
            }
            Input::Key(chord) => {
                let events = chord_events(chord)?;
                require_foreground(snapshot)?;
                send(&events)
            }
        }
    }
}
