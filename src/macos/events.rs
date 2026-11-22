use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, CGScrollEventUnit,
};

use super::accessibility::Focus;
use crate::desktop::{Input, ScreenPoint, WindowState};

fn post(events: &[CFRetained<CGEvent>]) {
    for event in events {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event));
    }
}

fn location(point: ScreenPoint) -> Result<CGPoint> {
    let (x, y) = point.points()?;
    Ok(CGPoint::new(x, y))
}

fn click(point: ScreenPoint) -> Result<[CFRetained<CGEvent>; 2]> {
    let at = location(point)?;
    let build = |kind| {
        CGEvent::new_mouse_event(None, kind, at, CGMouseButton::Left)
            .context("Could not construct a Quartz mouse event.")
    };
    Ok([
        build(CGEventType::LeftMouseDown)?,
        build(CGEventType::LeftMouseUp)?,
    ])
}

fn keyboard(code: u16, pressed: bool, flags: CGEventFlags) -> Result<CFRetained<CGEvent>> {
    let event = CGEvent::new_keyboard_event(None, code, pressed)
        .context("Could not construct a Quartz keyboard event.")?;
    CGEvent::set_flags(Some(&event), flags);
    Ok(event)
}

fn literal(text: &str) -> Result<Vec<CFRetained<CGEvent>>> {
    ensure!(
        !text.is_empty()
            && text.chars().take(2001).count() <= 2000
            && !text.chars().any(char::is_control),
        "Provide 1–2000 characters of plain text. Use press_key for control keys."
    );
    let mut events = Vec::new();
    for ch in text.chars() {
        let mut utf16 = [0; 2];
        let units = ch.encode_utf16(&mut utf16);
        let down = keyboard(0, true, CGEventFlags::empty())?;
        // Quartz copies the units during the call. Keep each surrogate pair in one event.
        unsafe {
            CGEvent::keyboard_set_unicode_string(Some(&down), units.len() as u64, units.as_ptr())
        };
        events.push(down);
        events.push(keyboard(0, false, CGEventFlags::empty())?);
    }
    Ok(events)
}

fn key_code(key: &str) -> Option<u16> {
    // ANSI positions for shortcuts. Literal text goes through Unicode events instead.
    let value = match key {
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "6" => 22,
        "5" => 23,
        "9" => 25,
        "7" => 26,
        "8" => 28,
        "0" => 29,
        "o" => 31,
        "u" => 32,
        "i" => 34,
        "p" => 35,
        "l" => 37,
        "j" => 38,
        "k" => 40,
        "n" => 45,
        "m" => 46,
        "enter" => 36,
        "tab" => 48,
        "space" => 49,
        "backspace" => 51,
        "escape" => 53,
        "home" => 115,
        "pageup" => 116,
        "delete" => 117,
        "end" => 119,
        "pagedown" => 121,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        _ => return None,
    };
    Some(value)
}

fn chord(value: &str) -> Result<[CFRetained<CGEvent>; 2]> {
    let lower = value.to_ascii_lowercase();
    let mut keys = lower.split('+').map(str::trim).collect::<Vec<_>>();
    let key = keys.pop().unwrap_or_default();
    let code = key_code(key).with_context(|| format!("Unsupported macOS key: {key:?}"))?;
    let mut flags = CGEventFlags::empty();
    for modifier in keys {
        let flag = match modifier {
            "ctrl" | "control" => CGEventFlags::MaskControl,
            "shift" => CGEventFlags::MaskShift,
            "alt" | "option" => CGEventFlags::MaskAlternate,
            "cmd" | "command" => CGEventFlags::MaskCommand,
            _ => bail!("Unsupported macOS modifier: {modifier:?}"),
        };
        ensure!(!flags.contains(flag), "Modifiers must not repeat.");
        flags.insert(flag);
    }
    Ok([keyboard(code, true, flags)?, keyboard(code, false, flags)?])
}

pub(super) fn apply(snapshot: &WindowState, initial: &Focus, input: Input<'_>) -> Result<()> {
    let focus = Focus::read()?;
    initial.verify_same_window(&focus)?;
    match input {
        Input::Click { point } => {
            ensure!(
                snapshot.bounds.contains(point),
                "The point is outside the captured window."
            );
            let events = click(point)?;
            focus.hit(point)?;
            focus.verify_same_window(&Focus::read()?)?;
            post(&events);
        }
        Input::Scroll { point, ticks } => {
            ensure!(
                (-30..=30).contains(&ticks) && ticks != 0,
                "Use a nonzero scroll count between -30 and 30."
            );
            ensure!(
                snapshot.bounds.contains(point),
                "The point is outside the captured window."
            );
            focus.hit(point)?;
            let event =
                CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 1, ticks, 0, 0)
                    .context("Could not construct a Quartz scroll event.")?;
            CGEvent::set_location(Some(&event), location(point)?);
            focus.verify_same_window(&Focus::read()?)?;
            post(&[event]);
        }
        Input::Fill {
            point,
            text,
            replace,
        } => {
            let letters = literal(text)?;
            let select = replace.then(|| chord("cmd+a")).transpose()?;
            ensure!(
                snapshot.bounds.contains(point),
                "The point is outside the captured window."
            );
            let events = click(point)?;
            let hit = focus.hit(point)?;
            focus.verify_same_window(&Focus::read()?)?;
            post(&events);
            let expires = Instant::now() + Duration::from_millis(400);
            loop {
                let current = Focus::read()?;
                focus.verify_same_window(&current)?;
                match current.require_text_focus(Some(&hit)) {
                    Ok(()) => break,
                    Err(error) if Instant::now() >= expires => return Err(error),
                    Err(_) => thread::sleep(Duration::from_millis(20)),
                }
            }
            if let Some(select) = &select {
                post(select);
            }
            post(&letters);
        }
        Input::Text(text) => {
            let letters = literal(text)?;
            let current = Focus::read()?;
            focus.verify_same_window(&current)?;
            current.require_text_focus(None)?;
            post(&letters);
        }
        Input::Key(value) => {
            let events = chord(value)?;
            focus.verify_same_window(&Focus::read()?)?;
            post(&events);
        }
    }
    Ok(())
}
