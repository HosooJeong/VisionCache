use std::ptr::{NonNull, null};

use anyhow::{Context, Result, ensure};
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType, CGPoint, CGSize};

use super::Candidate;
use crate::desktop::ScreenPoint;

pub(super) fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

fn attribute(element: &AXUIElement, name: &str) -> Option<CFRetained<CFType>> {
    let attribute = CFString::from_str(name);
    let mut value = null();
    let result = unsafe { element.copy_attribute_value(&attribute, NonNull::from(&mut value)) };
    if result != AXError::Success {
        return None;
    }
    // Copy-attribute gives the client one ownership reference.
    Some(unsafe { CFRetained::from_raw(NonNull::new(value.cast_mut())?) })
}

fn ax(element: &AXUIElement, name: &str) -> Option<CFRetained<AXUIElement>> {
    attribute(element, name)?.downcast().ok()
}

fn text(element: &AXUIElement, name: &str) -> Option<String> {
    attribute(element, name)?
        .downcast::<CFString>()
        .ok()
        .map(|s| s.to_string())
}

fn pid(element: &AXUIElement) -> Option<u32> {
    let mut value = 0;
    (unsafe { element.pid(NonNull::from(&mut value)) } == AXError::Success)
        .then(|| u32::try_from(value).ok())
        .flatten()
}

fn coordinates(element: &AXUIElement) -> Option<[f64; 4]> {
    let position = attribute(element, "AXPosition")?
        .downcast::<AXValue>()
        .ok()?;
    let size = attribute(element, "AXSize")?.downcast::<AXValue>().ok()?;
    let mut p = CGPoint::new(0.0, 0.0);
    let mut s = CGSize::new(0.0, 0.0);
    let valid = unsafe {
        position.value(AXValueType::CGPoint, NonNull::from(&mut p).cast())
            && size.value(AXValueType::CGSize, NonNull::from(&mut s).cast())
    };
    let bounds = [p.x, p.y, p.x + s.width, p.y + s.height];
    (valid && s.width > 0.0 && s.height > 0.0 && bounds.iter().all(|v| v.is_finite()))
        .then_some(bounds)
}

fn ancestors(element: CFRetained<AXUIElement>) -> impl Iterator<Item = CFRetained<AXUIElement>> {
    std::iter::successors(Some(element), |current| {
        let parent = ax(current, "AXParent")?;
        (*parent != **current).then_some(parent)
    })
    .take(24)
}

fn inside(element: &CFRetained<AXUIElement>, window: &CFRetained<AXUIElement>) -> bool {
    if **element == **window {
        return true;
    }
    if ax(element, "AXWindow").is_some_and(|ancestor| *ancestor == **window) {
        return true;
    }
    ancestors(element.clone()).any(|ancestor| *ancestor == **window)
}

struct Identity {
    process_id: u32,
    bounds: [f64; 4],
    title: Option<String>,
}

impl Identity {
    fn from_element(element: &AXUIElement) -> Option<Self> {
        Some(Self {
            process_id: pid(element)?,
            bounds: coordinates(element)?,
            title: text(element, "AXTitle").filter(|title| !title.trim().is_empty()),
        })
    }

    fn matches(&self, target: &Candidate) -> bool {
        self.process_id == target.window.process_id
            && self
                .bounds
                .iter()
                .zip(target.bounds)
                .all(|(a, b)| (a - b).abs() <= 1.0)
            && match (&self.title, &target.exact_title) {
                (Some(ax), Some(sc)) => ax == sc,
                _ => true,
            }
    }
}

pub(super) struct Focus {
    system: CFRetained<AXUIElement>,
    app: CFRetained<AXUIElement>,
    window: CFRetained<AXUIElement>,
    identity: Identity,
    sheet: bool,
}

impl Focus {
    pub(super) fn read() -> Result<Self> {
        ensure!(
            trusted(),
            "Accessibility permission is required to verify the foreground window."
        );
        let system = unsafe { AXUIElement::new_system_wide() };
        let _ = unsafe { system.set_messaging_timeout(0.6) };
        let app = ax(&system, "AXFocusedApplication")
            .context("macOS did not report a focused application.")?;
        let _ = unsafe { app.set_messaging_timeout(0.6) };
        let focused = ax(&app, "AXFocusedWindow")
            .context("The focused application has no accessible window.")?;
        let focus_is_sheet = text(&focused, "AXRole").is_some_and(|role| role == "AXSheet");
        let window = if focus_is_sheet {
            ancestors(focused.clone())
                .skip(1)
                .find(|parent| text(parent, "AXRole").is_some_and(|role| role == "AXWindow"))
                .unwrap_or(focused)
        } else {
            focused
        };
        let identity = Identity::from_element(&window)
            .context("The focused window has no reliable geometry.")?;
        // Some apps report the parent window while focus lives in a child sheet.
        let sheet = focus_is_sheet
            || ax(&app, "AXFocusedUIElement").is_some_and(|element| {
                inside(&element, &window)
                    && ancestors(element)
                        .any(|item| text(&item, "AXRole").is_some_and(|role| role == "AXSheet"))
            })
            || attribute(&window, "AXSheets")
                .and_then(|value| value.downcast::<CFArray>().ok())
                .is_some_and(|array| !array.is_empty());
        Ok(Self {
            system,
            app,
            window,
            identity,
            sheet,
        })
    }

    pub(super) fn has_sheet(&self) -> bool {
        self.sheet
    }

    pub(super) fn matches(&self, target: &Candidate) -> bool {
        self.identity.matches(target)
    }

    pub(super) fn match_window<'r>(&self, windows: &'r [Candidate]) -> Option<&'r Candidate> {
        let mut possible = windows
            .iter()
            .filter(|candidate| self.identity.matches(candidate));
        let first = possible.next()?;
        possible.next().is_none().then_some(first)
    }

    pub(super) fn belongs_to_window(&self, parent: &Candidate) -> bool {
        // An app can have many unrelated foreground windows with the same PID.
        // Only follow a window whose AX parent chain actually identifies this root.
        ancestors(self.window.clone()).skip(1).any(|element| {
            text(&element, "AXRole").is_some_and(|role| role == "AXWindow")
                && Identity::from_element(&element).is_some_and(|identity| identity.matches(parent))
        })
    }

    pub(super) fn verify_same_window(&self, updated: &Focus) -> Result<()> {
        ensure!(
            *self.window == *updated.window
                && self.sheet == updated.sheet
                && self.identity.process_id == updated.identity.process_id
                && self.identity.bounds == updated.identity.bounds,
            "The foreground window or modal sheet changed. Take another observation."
        );
        Ok(())
    }

    fn allowed(&self, element: &CFRetained<AXUIElement>) -> bool {
        pid(element) == Some(self.identity.process_id)
            && inside(element, &self.window)
            && (!self.sheet
                || ancestors(element.clone())
                    .any(|parent| text(&parent, "AXRole").is_some_and(|role| role == "AXSheet")))
    }

    pub(super) fn hit(&self, point: ScreenPoint) -> Result<CFRetained<AXUIElement>> {
        let (x, y) = point.points()?;
        let mut value = null();
        let status = unsafe {
            self.system
                .copy_element_at_position(x as f32, y as f32, NonNull::from(&mut value))
        };
        ensure!(
            status == AXError::Success,
            "macOS could not identify the element at that point."
        );
        let value = NonNull::new(value.cast_mut())
            .context("No Accessibility element was found at that point.")?;
        let element = unsafe { CFRetained::from_raw(value) };
        ensure!(
            self.allowed(&element),
            "The point is covered, outside the focused window, or outside its active sheet."
        );
        Ok(element)
    }

    pub(super) fn require_text_focus(
        &self,
        clicked: Option<&CFRetained<AXUIElement>>,
    ) -> Result<()> {
        let focus = ax(&self.app, "AXFocusedUIElement")
            .context("No Accessibility element has keyboard focus.")?;
        ensure!(
            self.allowed(&focus),
            "Keyboard focus is outside the observed window or active sheet."
        );
        if let Some(hit) = clicked {
            let role = text(&focus, "AXRole");
            ensure!(
                matches!(
                    role.as_deref(),
                    Some("AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField")
                ),
                "The clicked element did not focus an accessible text field."
            );
            ensure!(
                inside(hit, &focus) || inside(&focus, hit),
                "The focused text field does not match the clicked target."
            );
        }
        Ok(())
    }
}
