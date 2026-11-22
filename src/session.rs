use anyhow::{Context, Result, ensure};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    desktop::{Desktop, Input, ScreenBounds, ScreenPoint, Window, WindowState},
    image::{Mode, PngEncoder, Raster, View, views},
};

#[derive(Clone, Debug, Serialize)]
pub struct ImageInfo {
    index: usize,
    size: [u32; 2],
    source_box: [u32; 4],
    role: &'static str,
    reason: Option<&'static str>,
}

#[derive(Serialize)]
pub struct Observation {
    pub observation_id: String,
    requested_window: u64,
    window: Window,
    window_bounds: ScreenBounds,
    coordinate_space: &'static str,
    mode: Mode,
    effective_mode: Mode,
    images: Vec<ImageInfo>,
}

pub struct Observed {
    pub metadata: Observation,
    pub pngs: Vec<Vec<u8>>,
}

struct Snapshot {
    id: String,
    state: WindowState,
    size: [u32; 2],
    views: Vec<View>,
}

impl Snapshot {
    fn map(&self, image: usize, x: u32, y: u32) -> Result<ScreenPoint> {
        let view = self
            .views
            .get(image)
            .context("Image index is outside the observation.")?;
        ensure!(
            x < view.size[0] && y < view.size[1],
            "Point is outside the supplied image."
        );
        let [crop_width, crop_height] = view.rect.size();
        let point = match self.state.bounds {
            ScreenBounds::Pixels([left, top, right, bottom]) => {
                // Preserve Windows' integer projection for existing clients.
                let axis = |offset, crop_origin, crop_size, view_size, frame_size, lo, hi| {
                    let mapped = u64::from(crop_origin)
                        + u64::from(offset) * u64::from(crop_size) / u64::from(view_size);
                    let extent = u64::try_from(i64::from(hi) - i64::from(lo))?;
                    let translated = i64::try_from(mapped * extent / u64::from(frame_size))?;
                    i32::try_from(i64::from(lo) + translated)
                        .context("Mapped screen point is out of range.")
                };
                ScreenPoint::Pixels {
                    x: axis(
                        x,
                        view.rect.left,
                        crop_width,
                        view.size[0],
                        self.size[0],
                        left,
                        right,
                    )?,
                    y: axis(
                        y,
                        view.rect.top,
                        crop_height,
                        view.size[1],
                        self.size[1],
                        top,
                        bottom,
                    )?,
                }
            }
            ScreenBounds::Points([left, top, right, bottom]) => {
                let axis = |offset, crop_origin, crop_size, view_size, frame_size, lo, hi| {
                    let mapped = f64::from(crop_origin)
                        + f64::from(offset) * f64::from(crop_size) / f64::from(view_size);
                    lo + mapped * (hi - lo) / f64::from(frame_size)
                };
                ScreenPoint::Points {
                    x: axis(
                        x,
                        view.rect.left,
                        crop_width,
                        view.size[0],
                        self.size[0],
                        left,
                        right,
                    ),
                    y: axis(
                        y,
                        view.rect.top,
                        crop_height,
                        view.size[1],
                        self.size[1],
                        top,
                        bottom,
                    ),
                }
            }
        };
        ensure!(
            self.state.bounds.contains(point),
            "Mapped point is outside the captured surface."
        );
        Ok(point)
    }
}

pub struct Session {
    desktop: Box<dyn Desktop>,
    scale: f64,
    front: Raster,
    scratch: Raster,
    previous: Option<WindowState>,
    latest: Option<Snapshot>,
    encoder: PngEncoder,
}

impl Session {
    pub fn new(desktop: Box<dyn Desktop>, scale: f64) -> Result<Self> {
        ensure!(
            (0.1..1.0).contains(&scale),
            "Scale must be at least 0.1 and less than 1."
        );
        Ok(Self {
            desktop,
            scale,
            front: Raster::default(),
            scratch: Raster::default(),
            previous: None,
            latest: None,
            encoder: PngEncoder::default(),
        })
    }

    pub fn windows(&self) -> Result<Vec<Window>> {
        self.desktop.windows()
    }

    pub fn observe(&mut self, handle: u64, mode: Mode, follow_owned: bool) -> Result<Observed> {
        // Even if capture or encoding fails, the old coordinates may no longer be safe.
        self.latest = None;
        let state = self
            .desktop
            .capture(handle, follow_owned, &mut self.scratch)?;
        let previous = self
            .previous
            .as_ref()
            .filter(|previous| state.same_placement(previous))
            .map(|_| &self.front);
        let effective_mode = if mode == Mode::AdaptiveDelta
            && (state.window.handle != handle || state.attached_sheet)
        {
            Mode::Full
        } else {
            mode
        };
        let views = views(&self.scratch, previous, effective_mode, self.scale);
        let pngs = views
            .iter()
            .map(|&view| self.encoder.encode(&self.scratch, view))
            .collect::<Result<Vec<_>>>()?;
        let observation_id = Uuid::new_v4().simple().to_string();
        let images = views
            .iter()
            .enumerate()
            .map(|(index, view)| ImageInfo {
                index,
                size: view.size,
                source_box: view.rect.coords(),
                role: if index == 0 {
                    "Full view"
                } else {
                    "Changed region"
                },
                reason: view.reason,
            })
            .collect();

        std::mem::swap(&mut self.front, &mut self.scratch);
        self.previous = Some(state.clone());
        self.latest = Some(Snapshot {
            id: observation_id.clone(),
            state: state.clone(),
            size: self.front.size(),
            views,
        });
        Ok(Observed {
            metadata: Observation {
                observation_id,
                requested_window: handle,
                window: state.window,
                window_bounds: state.bounds,
                coordinate_space: state.bounds.unit(),
                mode,
                effective_mode,
                images,
            },
            pngs,
        })
    }

    fn current(&self, id: &str) -> Result<&Snapshot> {
        self.latest
            .as_ref()
            .filter(|snapshot| snapshot.id == id)
            .context("Observation is stale. Capture the target window again.")
    }

    pub fn map(&self, id: &str, image: usize, x: u32, y: u32) -> Result<ScreenPoint> {
        self.current(id)?.map(image, x, y)
    }

    pub fn input(&mut self, id: &str, input: Input<'_>) -> Result<()> {
        let state = self.current(id)?.state.clone();
        self.latest = None;
        self.desktop.input(&state, input)
    }
}
