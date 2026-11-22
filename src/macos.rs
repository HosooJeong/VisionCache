//! ScreenCaptureKit capture with a Quartz-coordinate Accessibility input guard.

mod accessibility;
mod events;

use anyhow::{Context, Result, ensure};
use block2::RcBlock;
use objc2::{AnyThread, rc::autoreleasepool};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, CGPreflightPostEventAccess, CGPreflightScreenCaptureAccess,
    kCGColorSpaceSRGB,
};
use objc2_foundation::{NSError, NSProcessInfo};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};
use serde::Serialize;
use std::{
    ptr::NonNull,
    sync::mpsc::{SyncSender, sync_channel},
    time::Duration,
};

use crate::{
    desktop::{Desktop, Input, ScreenBounds, Window, WindowState},
    image::Raster,
};
use accessibility::Focus;

const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Serialize)]
pub struct Permissions {
    pub screen_recording: bool,
    pub accessibility: bool,
    pub event_posting: bool,
}

pub fn permissions() -> Permissions {
    Permissions {
        screen_recording: CGPreflightScreenCaptureAccess(),
        accessibility: accessibility::trusted(),
        event_posting: CGPreflightPostEventAccess(),
    }
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub(super) window: Window,
    pub(super) exact_title: Option<String>,
    pub(super) bounds: [f64; 4],
}

impl Candidate {
    fn from_sc(window: &SCWindow) -> Option<Self> {
        // All of these properties are read while the framework owns this SCWindow.
        if !unsafe { window.isOnScreen() } {
            return None;
        }
        let owner = unsafe { window.owningApplication() }?;
        let process_id = u32::try_from(unsafe { owner.processID() }).ok()?;
        let handle = u64::from(unsafe { window.windowID() });
        let rect = unsafe { window.frame() };
        let bounds = rect_bounds(rect)?;
        if process_id == 0 || handle == 0 {
            return None;
        }
        let exact_title = unsafe { window.title() }
            .map(|title| title.to_string())
            .filter(|title| !title.trim().is_empty());
        let title = exact_title
            .clone()
            .unwrap_or_else(|| unsafe { owner.applicationName() }.to_string());
        Some(Self {
            window: Window {
                handle,
                process_id,
                thread_id: None,
                title,
            },
            exact_title,
            bounds,
        })
    }

    fn state(&self, size: [u32; 2], attached_sheet: bool) -> WindowState {
        WindowState {
            window: self.window.clone(),
            bounds: ScreenBounds::Points(self.bounds),
            raster_size: Some(size),
            attached_sheet,
        }
    }
}

fn rect_bounds(rect: CGRect) -> Option<[f64; 4]> {
    let (x, y) = (rect.origin.x, rect.origin.y);
    let (width, height) = (rect.size.width, rect.size.height);
    let result = [x, y, x + width, y + height];
    (width > 0.0 && height > 0.0 && result.iter().all(|v| v.is_finite())).then_some(result)
}

fn describe_error(error: *mut NSError, fallback: &str) -> anyhow::Error {
    match unsafe { error.as_ref() } {
        Some(error) => anyhow::anyhow!("{fallback}: {}", error.localizedDescription()),
        None => anyhow::anyhow!("{fallback}"),
    }
}

// The callback only sends Rust data or a thread-safe retained CGImage across threads.
// Start the screenshot asynchronously from the callback and wait on the caller only.
fn with_content<T: Send + 'static>(
    operation: impl Fn(&SCShareableContent, &SyncSender<Result<T>>) -> Result<()>
    + Send
    + Sync
    + 'static,
) -> Result<T> {
    ensure!(
        CGPreflightScreenCaptureAccess(),
        "Grant Screen Recording permission to the MCP host in macOS Privacy & Security."
    );
    autoreleasepool(|_| {
        let (send, receive) = sync_channel(1);
        let completion = RcBlock::new(
            move |content: *mut SCShareableContent, error: *mut NSError| {
                let result = match unsafe { content.as_ref() } {
                    Some(content) => operation(content, &send),
                    None => Err(describe_error(
                        error,
                        "ScreenCaptureKit could not list windows",
                    )),
                };
                if let Err(error) = result {
                    let _ = send.try_send(Err(error));
                }
            },
        );
        unsafe {
            SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
                true, true, &completion,
            );
        }
        receive
            .recv_timeout(TIMEOUT)
            .context("ScreenCaptureKit did not answer in time.")?
    })
}

fn candidates() -> Result<Vec<Candidate>> {
    with_content(|content, sender| {
        let windows = unsafe { content.windows() };
        let windows = windows
            .to_vec()
            .iter()
            .filter_map(|window| Candidate::from_sc(window))
            .collect();
        let _ = sender.try_send(Ok(windows));
        Ok(())
    })
}

fn selected(
    windows: &[Candidate],
    requested: u64,
    follow_owned: bool,
) -> Result<(&Candidate, bool)> {
    let root = windows
        .iter()
        .find(|w| w.window.handle == requested)
        .context("The selected on-screen window is no longer available.")?;
    let Ok(focus) = Focus::read() else {
        return Ok((root, false));
    };
    let Some(front) = focus.match_window(windows) else {
        return Ok((root, false));
    };
    if front.window.handle == requested {
        return Ok((root, focus.has_sheet()));
    }
    if follow_owned
        && front.window.process_id == root.window.process_id
        && focus.belongs_to_window(root)
    {
        return Ok((front, focus.has_sheet()));
    }
    Ok((root, false))
}

fn pixel_size(filter: &SCContentFilter, target: &Candidate) -> Result<[u32; 2]> {
    let content = unsafe { filter.contentRect() };
    let scale = f64::from(unsafe { filter.pointPixelScale() });
    let width = content.size.width;
    let height = content.size.height;
    let [l, t, r, b] = target.bounds;
    ensure!(
        scale.is_finite() && scale > 0.0,
        "Invalid ScreenCaptureKit pixel scale."
    );
    ensure!(
        (width - (r - l)).abs() <= 1.0 && (height - (b - t)).abs() <= 1.0,
        "Capture content and window geometry do not agree."
    );
    let pixels = [(width * scale).round(), (height * scale).round()];
    ensure!(
        pixels
            .iter()
            .all(|n| (1.0..=f64::from(u32::MAX)).contains(n)),
        "The capture dimensions are out of range."
    );
    Ok([pixels[0] as u32, pixels[1] as u32])
}

fn on_sc_window<T: Send + 'static>(
    target: Candidate,
    operation: impl Fn(&SCContentFilter, [u32; 2], &SyncSender<Result<T>>) -> Result<()>
    + Send
    + Sync
    + 'static,
) -> Result<T> {
    with_content(move |content, sender| {
        let windows = unsafe { content.windows() };
        let window = windows
            .to_vec()
            .into_iter()
            .find(|window| {
                Candidate::from_sc(window).is_some_and(|value| {
                    value.window.handle == target.window.handle
                        && value.window.process_id == target.window.process_id
                        && value.bounds == target.bounds
                })
            })
            .context("The window moved, closed, or changed ownership. Observe it again.")?;
        let filter = unsafe {
            SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
        };
        let size = pixel_size(&filter, &target)?;
        operation(&filter, size, sender)
    })
}

fn screenshot(target: Candidate) -> Result<(CFRetained<CGImage>, [u32; 2])> {
    on_sc_window(target, |filter, size, sender| {
        let config = unsafe { SCStreamConfiguration::new() };
        unsafe {
            config.setWidth(size[0] as usize);
            config.setHeight(size[1] as usize);
            config.setShowsCursor(false);
            config.setIgnoreShadowsSingleWindow(true);
        }
        let sender = sender.clone();
        let done = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
            let result = match NonNull::new(image) {
                // The callback's +0 CGImage is retained so the caller may use it.
                Some(image) => Ok((unsafe { CFRetained::retain(image) }, size)),
                None => Err(describe_error(
                    error,
                    "ScreenCaptureKit did not return an image",
                )),
            };
            let _ = sender.try_send(result);
        });
        unsafe {
            SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                filter,
                &config,
                Some(&done),
            );
        }
        Ok(())
    })
}

#[derive(Default)]
pub struct MacDesktop {
    bgrx: Vec<u8>,
}

impl MacDesktop {
    pub fn new() -> Result<Self> {
        let version = NSProcessInfo::processInfo().operatingSystemVersion();
        ensure!(
            version.majorVersion >= 14,
            "VisionCache requires macOS 14 Sonoma or later."
        );
        Ok(Self::default())
    }

    fn normalize(&mut self, cg: &CGImage, size: [u32; 2], frame: &mut Raster) -> Result<()> {
        let [width, height] = size;
        ensure!(
            CGImage::width(Some(cg)) == width as usize
                && CGImage::height(Some(cg)) == height as usize,
            "macOS returned a differently sized image. Observe the window again."
        );
        let row = (width as usize)
            .checked_mul(4)
            .context("Capture is too wide.")?;
        let total = row
            .checked_mul(height as usize)
            .context("Capture is too large.")?;
        self.bgrx
            .try_reserve(total.saturating_sub(self.bgrx.len()))?;
        self.bgrx.resize(total, 0);
        self.bgrx.fill(0);

        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .context("Could not create the screenshot color space.")?;
        {
            let layout = CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::NoneSkipFirst.0;
            // The bitmap context cannot outlive this scope or reallocate the backing vector.
            let context = unsafe {
                CGBitmapContextCreate(
                    self.bgrx.as_mut_ptr().cast(),
                    width as usize,
                    height as usize,
                    8,
                    row,
                    Some(&space),
                    layout,
                )
            }
            .context("Could not create a macOS bitmap context.")?;
            CGContext::draw_image(
                Some(&context),
                CGRect::new(
                    CGPoint::new(0.0, 0.0),
                    CGSize::new(width as f64, height as f64),
                ),
                Some(cg),
            );
        }
        let packed = frame.prepare(width, height)?;
        let padded_stride = (width as usize * 3 + 3) & !3;
        for (src, dest) in self
            .bgrx
            .chunks_exact(row)
            .zip(packed.chunks_exact_mut(padded_stride))
        {
            for (pixel, out) in src.chunks_exact(4).zip(dest.chunks_exact_mut(3)) {
                out.copy_from_slice(&pixel[..3]);
            }
        }
        Ok(())
    }
}

impl Desktop for MacDesktop {
    fn windows(&self) -> Result<Vec<Window>> {
        Ok(candidates()?
            .into_iter()
            .map(|candidate| candidate.window)
            .collect())
    }

    fn capture(
        &mut self,
        requested: u64,
        follow_owned: bool,
        frame: &mut Raster,
    ) -> Result<WindowState> {
        let list = candidates()?;
        let (target, sheet) = selected(&list, requested, follow_owned)?;
        let (image, size) = screenshot(target.clone())?;
        let state = target.state(size, sheet);
        let current = candidates()?;
        let (current_target, current_sheet) = selected(&current, requested, follow_owned)?;
        ensure!(
            state.same_placement(&current_target.state(size, current_sheet)),
            "The active window or modal changed during capture. Observe it again."
        );
        self.normalize(&image, size, frame)?;
        Ok(state)
    }

    fn input(&mut self, snapshot: &WindowState, input: Input<'_>) -> Result<()> {
        let access = permissions();
        ensure!(
            access.accessibility && access.event_posting,
            "Grant Accessibility permission to the MCP host in macOS Privacy & Security before sending input."
        );
        let list = candidates()?;
        let initial_focus = Focus::read()?;
        let target = initial_focus
            .match_window(&list)
            .context("The foreground Accessibility window could not be matched to a capture.")?;
        ensure!(
            target.window.handle == snapshot.window.handle,
            "Focus the observed window and capture it again before sending input."
        );
        let size = on_sc_window(target.clone(), |_, size, sender| {
            let _ = sender.try_send(Ok(size));
            Ok(())
        })?;
        let focus = Focus::read()?;
        initial_focus.verify_same_window(&focus)?;
        ensure!(
            focus.matches(target)
                && snapshot.same_placement(&target.state(size, focus.has_sheet())),
            "The window placement, focus, or sheet changed. Take another observation."
        );

        events::apply(snapshot, &focus, input)
    }
}
