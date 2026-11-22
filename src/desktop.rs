use anyhow::{Result, bail};
use serde::Serialize;

use crate::image::Raster;

#[derive(Clone, Debug, Serialize)]
pub struct Window {
    pub handle: u64,
    pub process_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<u32>,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ScreenBounds {
    Pixels([i32; 4]),
    Points([f64; 4]),
}

impl ScreenBounds {
    pub fn unit(self) -> &'static str {
        match self {
            Self::Pixels(_) => "screen_pixels",
            Self::Points(_) => "screen_points",
        }
    }

    pub fn contains(self, point: ScreenPoint) -> bool {
        match (self, point) {
            (Self::Pixels([l, t, r, b]), ScreenPoint::Pixels { x, y }) => {
                l <= x && x < r && t <= y && y < b
            }
            (Self::Points([l, t, r, b]), ScreenPoint::Points { x, y }) => {
                l <= x && x < r && t <= y && y < b
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ScreenPoint {
    Pixels { x: i32, y: i32 },
    Points { x: f64, y: f64 },
}

impl ScreenPoint {
    pub fn pixels(self) -> Result<(i32, i32)> {
        match self {
            Self::Pixels { x, y } => Ok((x, y)),
            _ => bail!("Expected a physical screen pixel coordinate."),
        }
    }

    pub fn points(self) -> Result<(f64, f64)> {
        match self {
            Self::Points { x, y } => Ok((x, y)),
            _ => bail!("Expected a logical screen point coordinate."),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WindowState {
    pub window: Window,
    /// Left, top, right, bottom in the platform's global coordinate space.
    pub bounds: ScreenBounds,
    /// The raster size expected from display scale and capture configuration.
    pub raster_size: Option<[u32; 2]>,
    /// A modal sheet drawn inside the requested window's client image.
    pub attached_sheet: bool,
}

impl WindowState {
    pub fn same_placement(&self, other: &Self) -> bool {
        self.window.handle == other.window.handle
            && self.window.process_id == other.window.process_id
            && self.window.thread_id == other.window.thread_id
            && self.bounds == other.bounds
            && self.raster_size == other.raster_size
            && self.attached_sheet == other.attached_sheet
    }
}

pub enum Input<'a> {
    Click {
        point: ScreenPoint,
    },
    Scroll {
        point: ScreenPoint,
        ticks: i32,
    },
    Fill {
        point: ScreenPoint,
        text: &'a str,
        replace: bool,
    },
    Text(&'a str),
    Key(&'a str),
}

pub trait Desktop: Send {
    fn windows(&self) -> Result<Vec<Window>>;
    fn capture(
        &mut self,
        requested: u64,
        follow_owned: bool,
        pixels: &mut Raster,
    ) -> Result<WindowState>;
    fn input(&mut self, snapshot: &WindowState, input: Input<'_>) -> Result<()>;
}
