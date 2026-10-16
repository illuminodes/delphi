use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use yew::UseStateHandle;

use super::frame_loop::FrameLoop;
use super::oracle_painter::OraclePainter;
use crate::{OracleScene, OracleSketch};

pub struct OracleStage {
    _frames: FrameLoop,
}

impl OracleStage {
    const MAX_RATIO: f64 = 2.0;
    const LONGEST_STEP: f64 = 0.25;

    pub fn start(canvas: HtmlCanvasElement, size: (f64, f64), scene: Rc<RefCell<OracleScene>>, newest: UseStateHandle<Option<u32>>) -> Option<Self> {
        let ratio = web_sys::window()?.device_pixel_ratio().clamp(1.0, Self::MAX_RATIO);
        let (width, height) = size;
        canvas.set_width((width * ratio).round() as u32);
        canvas.set_height((height * ratio).round() as u32);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        context.set_transform(ratio, 0.0, 0.0, ratio, 0.0, 0.0).ok()?;
        let painter = OraclePainter::new(context, ratio);
        scene.borrow_mut().resize(width, height);
        let mut last: Option<f64> = None;
        let frames = FrameLoop::start(move |now| {
            let step = last.map_or(0.0, |before| (now - before).min(Self::LONGEST_STEP));
            last = Some(now);
            let mut scene = scene.borrow_mut();
            scene.advance(step);
            painter.paint(&OracleSketch::of(&scene), width, height);
            let height = scene.wall().newest().map(|block| block.height);
            if *newest != height {
                newest.set(height);
            }
        })?;
        Some(Self { _frames: frames })
    }
}
