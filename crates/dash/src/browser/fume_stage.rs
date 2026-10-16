use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use yew::UseStateHandle;

use super::frame_loop::FrameLoop;
use super::fume_painter::FumePainter;
use crate::{FumeScene, FumeSketch, HarvestTimer};

pub struct FumeStage {
    _frames: FrameLoop,
}

impl FumeStage {
    const MAX_RATIO: f64 = 2.0;
    const TALLY_EVERY: f64 = 0.25;

    pub fn start(canvas: HtmlCanvasElement, size: (f64, f64), scene: Rc<RefCell<FumeScene>>, timer: Rc<RefCell<HarvestTimer>>, tally: UseStateHandle<Vec<usize>>) -> Option<Self> {
        let ratio = web_sys::window()?.device_pixel_ratio().clamp(1.0, Self::MAX_RATIO);
        let (width, height) = size;
        canvas.set_width((width * ratio).round() as u32);
        canvas.set_height((height * ratio).round() as u32);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        context.set_transform(ratio, 0.0, 0.0, ratio, 0.0, 0.0).ok()?;
        let painter = FumePainter::new(context);
        scene.borrow_mut().resize(width, height);
        let mut last: Option<f64> = None;
        let mut since_tally = 0.0;
        let frames = FrameLoop::start(move |now| {
            let step = last.map_or(0.0, |before| now - before);
            last = Some(now);
            let mut scene = scene.borrow_mut();
            if timer.borrow_mut().advance(step) {
                scene.harvest();
            }
            scene.advance(step);
            painter.paint(&FumeSketch::of(&scene), width, height);
            since_tally += step;
            if since_tally >= Self::TALLY_EVERY {
                since_tally = 0.0;
                let mut counts: Vec<usize> = scene.jars().iter().map(|jar| jar.collected()).collect();
                counts.push(scene.scattered());
                counts.push(scene.harvests());
                if *tally != counts {
                    tally.set(counts);
                }
            }
        })?;
        Some(Self { _frames: frames })
    }
}
