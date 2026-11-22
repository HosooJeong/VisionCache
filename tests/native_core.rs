use std::{
    io::Cursor,
    sync::{Arc, Mutex},
};

use anyhow::{Result, ensure};
use visioncache::{
    desktop::{Desktop, Input, ScreenBounds, Window, WindowState},
    image::{Mode, PngEncoder, Raster, views},
    session::Session,
};

fn solid(width: u32, height: u32, bgr: [u8; 3]) -> Raster {
    let mut raster = Raster::default();
    let stride = (width as usize * 3 + 3) & !3;
    for row in raster
        .prepare(width, height)
        .unwrap()
        .chunks_exact_mut(stride)
    {
        for pixel in row[..width as usize * 3].chunks_exact_mut(3) {
            pixel.copy_from_slice(&bgr);
        }
    }
    raster
}

fn with_glyphs(bgr: [u8; 3]) -> Raster {
    let [width, height] = [301, 200];
    let stride = (width as usize * 3 + 3) & !3;
    let mut image = Raster::default();
    let bytes = image.prepare(width, height).unwrap();
    bytes.fill(255);
    for x in (159..207).step_by(5) {
        for y in 85..94 {
            for column in x..x + 2 {
                let p = y * stride + column * 3;
                bytes[p..p + 3].copy_from_slice(&bgr);
            }
        }
    }
    image
}

#[test]
fn adaptive_selects_small_text_and_red_warnings() {
    let before = solid(301, 200, [255; 3]);
    for (ink, reason) in [
        ([69, 97, 33], "small_text"),
        ([46, 41, 174], "red_warning_text"),
    ] {
        let now = with_glyphs(ink);
        let frames = views(&now, Some(&before), Mode::AdaptiveDelta, 0.5);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].size, [150, 100]);
        assert_eq!(frames[1].reason, Some(reason));
        let [left, top, right, bottom] = frames[1].rect.coords();
        assert!(left <= 159 && top <= 85 && right >= 206 && bottom >= 94);
        assert_eq!(frames[1].size, frames[1].rect.size());
        assert_eq!(views(&now, Some(&now), Mode::AdaptiveDelta, 0.5).len(), 1);
    }
}

#[test]
fn padding_and_repaint_do_not_create_an_adaptive_crop() {
    let previous = solid(301, 200, [255; 3]);
    let mut now = solid(301, 200, [255; 3]);
    let stride = (301 * 3 + 3) & !3;
    for row in now.prepare(301, 200).unwrap().chunks_exact_mut(stride) {
        row[301 * 3..].fill(190);
    }
    assert_eq!(
        views(&now, Some(&previous), Mode::ResizedDelta, 0.5).len(),
        1
    );

    let repaint = solid(301, 200, [60, 75, 85]);
    assert_eq!(
        views(&repaint, Some(&previous), Mode::AdaptiveDelta, 0.5).len(),
        1
    );
    assert_eq!(
        views(&repaint, Some(&previous), Mode::ResizedDelta, 0.5).len(),
        1
    );
}

#[test]
fn generic_delta_keeps_local_graphics() {
    let before = solid(301, 200, [255; 3]);
    let mut repaint = solid(301, 200, [255; 3]);
    let stride = (301 * 3 + 3) & !3;
    let bytes = repaint.prepare(301, 200).unwrap();
    for row in 80..110 {
        bytes[row * stride + 30 * 3..row * stride + 55 * 3].fill(0);
    }
    assert_eq!(
        views(&repaint, Some(&before), Mode::AdaptiveDelta, 0.5).len(),
        1
    );
    assert_eq!(
        views(&repaint, Some(&before), Mode::ResizedDelta, 0.5).len(),
        2
    );
}

#[test]
fn png_decodes_with_correct_colors_and_sizes_at_odd_widths() {
    let raster = solid(153, 80, [231, 42, 17]);
    let mut encoder = PngEncoder::default();
    for mode in [Mode::Full, Mode::Resized] {
        let view = views(&raster, None, mode, 0.5)[0];
        let encoded = encoder.encode(&raster, view).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(encoded)).read_info().unwrap();
        let mut bytes = vec![0u8; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut bytes).unwrap();
        assert_eq!([frame.width, frame.height], view.size);
        assert!(
            bytes[..frame.buffer_size()]
                .chunks_exact(3)
                .all(|p| p == [17, 42, 231])
        );
    }
}

#[derive(Default)]
struct Fixture {
    dialog: bool,
    broken: bool,
    actions: usize,
}

struct Mock(Arc<Mutex<Fixture>>);

impl Desktop for Mock {
    fn windows(&self) -> Result<Vec<Window>> {
        Ok(vec![window(1)])
    }

    fn capture(
        &mut self,
        requested: u64,
        follow: bool,
        raster: &mut Raster,
    ) -> Result<WindowState> {
        let fixture = self.0.lock().unwrap();
        ensure!(!fixture.broken, "capture failed");
        let (handle, bounds) = if follow && fixture.dialog {
            (2, [0, 0, 300, 200])
        } else {
            (requested, [-200, 30, 400, 430])
        };
        raster.prepare(300, 200)?.fill(213);
        Ok(WindowState {
            window: window(handle),
            bounds: ScreenBounds::Pixels(bounds),
            raster_size: None,
            attached_sheet: false,
        })
    }

    fn input(&mut self, _: &WindowState, _: Input<'_>) -> Result<()> {
        self.0.lock().unwrap().actions += 1;
        Ok(())
    }
}

fn window(handle: u64) -> Window {
    Window {
        handle,
        process_id: 123,
        thread_id: Some(34),
        title: format!("Window {handle}"),
    }
}

#[test]
fn snapshot_coordinates_and_input_are_scoped_to_the_current_surface() {
    let shared = Arc::new(Mutex::new(Fixture::default()));
    let mut session = Session::new(Box::new(Mock(shared.clone())), 0.5).unwrap();

    let old = session.observe(1, Mode::AdaptiveDelta, true).unwrap();
    let point = session
        .map(&old.metadata.observation_id, 0, 75, 50)
        .unwrap();
    assert_eq!(point.pixels().unwrap(), (100, 230));
    assert!(
        session
            .map(&old.metadata.observation_id, 0, 150, 100)
            .is_err()
    );

    shared.lock().unwrap().dialog = true;
    let modal = session.observe(1, Mode::AdaptiveDelta, true).unwrap();
    assert!(session.map(&old.metadata.observation_id, 0, 0, 0).is_err());
    let info = serde_json::to_value(&modal.metadata).unwrap();
    assert_eq!(info["window"]["handle"], 2);
    assert_eq!(info["requested_window"], 1);
    assert_eq!(info["effective_mode"], "full");
    assert_eq!(info["images"][0]["size"], serde_json::json!([300, 200]));

    session
        .input(&modal.metadata.observation_id, Input::Text("한글"))
        .unwrap();
    assert!(
        session
            .input(&modal.metadata.observation_id, Input::Key("enter"))
            .is_err()
    );
    assert_eq!(shared.lock().unwrap().actions, 1);

    shared.lock().unwrap().dialog = false;
    let restored = session.observe(1, Mode::AdaptiveDelta, true).unwrap();
    let info = serde_json::to_value(&restored.metadata).unwrap();
    assert_eq!(info["window"]["handle"], 1);
    assert_eq!(info["effective_mode"], "adaptive_delta");
    assert_eq!(restored.pngs.len(), 1);

    shared.lock().unwrap().broken = true;
    assert!(session.observe(1, Mode::AdaptiveDelta, true).is_err());
    assert!(
        session
            .map(&restored.metadata.observation_id, 0, 0, 0)
            .is_err()
    );
}

#[test]
fn mac_geometry_preserves_negative_and_fractional_screen_points() {
    struct RetroDisplay;
    impl Desktop for RetroDisplay {
        fn windows(&self) -> Result<Vec<Window>> {
            Ok(vec![window(7)])
        }

        fn capture(&mut self, handle: u64, _: bool, raster: &mut Raster) -> Result<WindowState> {
            raster.prepare(800, 600)?.fill(255);
            Ok(WindowState {
                window: window(handle),
                bounds: ScreenBounds::Points([-700.5, 24.25, -300.5, 324.25]),
                raster_size: Some([800, 600]),
                attached_sheet: false,
            })
        }

        fn input(&mut self, _: &WindowState, _: Input<'_>) -> Result<()> {
            Ok(())
        }
    }

    let mut session = Session::new(Box::new(RetroDisplay), 0.5).unwrap();
    let full = session.observe(7, Mode::Full, false).unwrap();
    let mapped = session
        .map(&full.metadata.observation_id, 0, 5, 11)
        .unwrap()
        .points()
        .unwrap();
    assert_eq!(mapped, (-698.0, 29.75));
    let halfway = session.observe(7, Mode::Resized, false).unwrap();
    let mapped = session
        .map(&halfway.metadata.observation_id, 0, 10, 20)
        .unwrap()
        .points()
        .unwrap();
    assert_eq!(mapped, (-690.5, 44.25));
    let json = serde_json::to_value(halfway.metadata).unwrap();
    assert_eq!(json["coordinate_space"], "screen_points");
    assert_eq!(json["window_bounds"][0], -700.5);
}
