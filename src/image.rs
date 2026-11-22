use std::borrow::Cow;

use anyhow::{Context, Result, bail, ensure};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Full,
    Resized,
    ResizedDelta,
    #[default]
    AdaptiveDelta,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Rect {
    pub fn size(self) -> [u32; 2] {
        [self.right - self.left, self.bottom - self.top]
    }

    pub fn coords(self) -> [u32; 4] {
        [self.left, self.top, self.right, self.bottom]
    }

    fn pad(self, n: u32, [width, height]: [u32; 2]) -> Self {
        Self {
            left: self.left.saturating_sub(n),
            top: self.top.saturating_sub(n),
            right: self.right.saturating_add(n).min(width),
            bottom: self.bottom.saturating_add(n).min(height),
        }
    }

    fn union(self, b: Self) -> Self {
        Self {
            left: self.left.min(b.left),
            top: self.top.min(b.top),
            right: self.right.max(b.right),
            bottom: self.bottom.max(b.bottom),
        }
    }
}

/// Top-down BGR24 bytes with native DWORD row alignment. Padding is never compared.
#[derive(Default)]
pub struct Raster {
    size: [u32; 2],
    stride: usize,
    bytes: Vec<u8>,
}

impl Raster {
    pub fn prepare(&mut self, width: u32, height: u32) -> Result<&mut [u8]> {
        ensure!(
            width != 0 && height != 0,
            "Capture has an empty client area."
        );
        let row_bytes = (width as usize)
            .checked_mul(3)
            .context("Capture is too wide.")?;
        let stride = row_bytes.checked_add(3).context("Capture is too wide.")? & !3;
        let len = stride
            .checked_mul(height as usize)
            .context("Capture is too large.")?;
        self.bytes
            .try_reserve(len.saturating_sub(self.bytes.len()))?;
        self.bytes.resize(len, 0);
        self.size = [width, height];
        self.stride = stride;
        Ok(&mut self.bytes)
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn all_black(&self) -> bool {
        (0..self.size[1]).all(|y| self.row(y).iter().all(|&c| c == 0))
    }

    fn row(&self, y: u32) -> &[u8] {
        let start = y as usize * self.stride;
        &self.bytes[start..start + self.size[0] as usize * 3]
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let offset = y as usize * self.stride + x as usize * 3;
        [
            self.bytes[offset],
            self.bytes[offset + 1],
            self.bytes[offset + 2],
        ]
    }

    fn packed(&self) -> Cow<'_, [u8]> {
        if self.stride == self.size[0] as usize * 3 {
            return Cow::Borrowed(&self.bytes);
        }
        Cow::Owned(
            (0..self.size[1])
                .flat_map(|y| self.row(y).iter().copied())
                .collect(),
        )
    }
}

fn luma([b, g, r]: [u8; 3]) -> u32 {
    (7471 * u32::from(b) + 38470 * u32::from(g) + 19595 * u32::from(r) + 32768) >> 16
}

fn changed(a: [u8; 3], b: [u8; 3]) -> bool {
    luma([
        a[0].abs_diff(b[0]),
        a[1].abs_diff(b[1]),
        a[2].abs_diff(b[2]),
    ]) >= 18
}

fn diff_area(current: &Raster, previous: &Raster) -> Option<Rect> {
    let [width, height] = current.size;
    let mut result: Option<Rect> = None;
    for y in 0..height {
        let current_row = current.row(y);
        let previous_row = previous.row(y);
        if current_row == previous_row {
            continue;
        }
        let mut points = current_row
            .chunks_exact(3)
            .zip(previous_row.chunks_exact(3))
            .enumerate()
            .filter_map(|(x, (a, b))| {
                changed([a[0], a[1], a[2]], [b[0], b[1], b[2]]).then_some(x as u32)
            });
        if let Some(left) = points.next() {
            let right = points.next_back().unwrap_or(left) + 1;
            let line = Rect {
                left,
                top: y,
                right: right.min(width),
                bottom: y + 1,
            };
            result = Some(result.map_or(line, |old| old.union(line)));
        }
    }
    result
}

fn text_bands(area: Rect, ink: impl Fn(u32, u32) -> bool) -> Vec<Rect> {
    let mut bands = Vec::new();
    let mut band: Option<Rect> = None;
    for y in area.top..=area.bottom {
        let mut left = area.right;
        let mut right = area.left;
        let mut previous_ink = false;
        let (mut pixels, mut strokes) = (0, 0);
        if y < area.bottom {
            for x in area.left..area.right {
                let filled = ink(x, y);
                if filled {
                    left = left.min(x);
                    right = x + 1;
                    pixels += 1;
                    strokes += u32::from(!previous_ink);
                }
                previous_ink = filled;
            }
        }
        if pixels >= 7 && strokes >= 3 {
            let row = Rect {
                left,
                top: y,
                right,
                bottom: y + 1,
            };
            band = Some(band.map_or(row, |open| open.union(row)));
        } else if let Some(mut candidate) = band.take() {
            let [width, height] = candidate.size();
            if width < 24 || height < 5 {
                continue;
            }
            let has_ink = |row| (candidate.left..candidate.right).any(|x| ink(x, row));
            while candidate.top > area.top && has_ink(candidate.top - 1) {
                candidate.top -= 1;
            }
            while candidate.bottom < area.bottom && has_ink(candidate.bottom) {
                candidate.bottom += 1;
            }
            bands.push(candidate);
        }
    }
    bands
}

fn warning([b, g, r]: [u8; 3]) -> bool {
    r.saturating_sub(g) >= 75 && r.saturating_sub(b) >= 75 && g.abs_diff(b) <= 25
}

fn dark([b, g, r]: [u8; 3]) -> bool {
    b.saturating_sub(r) < 80 && luma([b, g, r]) < 158
}

fn detail(
    current: &Raster,
    previous: &Raster,
    area: Rect,
    scale: f64,
) -> Option<(Rect, &'static str)> {
    let area = area.pad(12, current.size);
    let red = text_bands(area, |x, y| {
        let pixel = current.pixel(x, y);
        warning(pixel) && changed(pixel, previous.pixel(x, y))
    });
    if let Some(rect) = red.into_iter().reduce(Rect::union) {
        return Some((rect, "red_warning_text"));
    }
    text_bands(area, |x, y| dark(current.pixel(x, y)))
        .into_iter()
        .filter(|band| {
            f64::from(band.size()[1]) * scale <= 6.0
                && (band.top..band.bottom)
                    .flat_map(|y| (band.left..band.right).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let pixel = current.pixel(x, y);
                        dark(pixel) && changed(pixel, previous.pixel(x, y))
                    })
                    .take(16)
                    .count()
                    == 16
        })
        .reduce(Rect::union)
        .map(|rect| (rect, "small_text"))
}

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub size: [u32; 2],
    pub rect: Rect,
    pub reason: Option<&'static str>,
}

pub fn views(current: &Raster, previous: Option<&Raster>, mode: Mode, scale: f64) -> Vec<View> {
    let [width, height] = current.size;
    let rect = Rect {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let scaled = |n| (f64::from(n) * scale).round_ties_even().max(1.0) as u32;
    let size = if mode == Mode::Full {
        current.size
    } else {
        [scaled(width), scaled(height)]
    };
    let mut views = vec![View {
        size,
        rect,
        reason: None,
    }];
    if !matches!(mode, Mode::ResizedDelta | Mode::AdaptiveDelta) {
        return views;
    }
    if let Some(previous) = previous.filter(|p| p.size == current.size)
        && let Some(difference) = diff_area(current, previous)
    {
        let candidate = if mode == Mode::ResizedDelta {
            Some((difference, "pixel_change"))
        } else {
            detail(current, previous, difference, scale)
        };
        if let Some((rect, reason)) = candidate {
            let rect = rect.pad(36, current.size);
            let [w, h] = rect.size();
            if u64::from(w) * u64::from(h) <= u64::from(width) * u64::from(height) / 4 {
                views.push(View {
                    size: rect.size(),
                    rect,
                    reason: Some(reason),
                });
            }
        }
    }
    views
}

#[derive(Default)]
pub struct PngEncoder {
    resize: Resizer,
}

impl PngEncoder {
    pub fn encode(&mut self, frame: &Raster, view: View) -> Result<Vec<u8>> {
        use std::io::Write;

        let [width, height] = view.size;
        let mut output = Vec::new();
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        let mut stream = writer.stream_writer()?;
        let mut rgb = vec![0; width as usize * 3];

        if view.size == view.rect.size() {
            for y in view.rect.top..view.rect.bottom {
                let row = &frame.row(y)[view.rect.left as usize * 3..view.rect.right as usize * 3];
                write_rgb(&mut stream, &mut rgb, row)?;
            }
        } else {
            if view.rect.coords() != [0, 0, frame.size[0], frame.size[1]] {
                bail!("Only the overview can be resized.");
            }
            let packed = frame.packed();
            let source =
                images::ImageRef::new(frame.size[0], frame.size[1], &packed, PixelType::U8x3)?;
            let mut resized = images::Image::new(width, height, PixelType::U8x3);
            self.resize.resize(
                &source,
                &mut resized,
                &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
            )?;
            for row in resized.buffer().chunks_exact(rgb.len()) {
                write_rgb(&mut stream, &mut rgb, row)?;
            }
        }
        stream.flush()?;
        stream.finish()?;
        writer.finish()?;
        Ok(output)
    }
}

fn write_rgb<W: std::io::Write>(output: &mut W, rgb: &mut [u8], bgr: &[u8]) -> std::io::Result<()> {
    for (dst, src) in rgb.chunks_exact_mut(3).zip(bgr.chunks_exact(3)) {
        dst.copy_from_slice(&[src[2], src[1], src[0]]);
    }
    output.write_all(rgb)
}
