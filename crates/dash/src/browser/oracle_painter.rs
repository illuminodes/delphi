use std::cell::RefCell;
use std::collections::HashMap;

use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

use super::fume_painter::FumePainter;
use super::stone_cutter::StoneCutter;
use crate::{FumeBowl, Limestone, OracleSketch, StoneSketch};

pub struct OraclePainter {
    context: CanvasRenderingContext2d,
    marks: FumePainter,
    photo: Option<HtmlImageElement>,
    scale: f64,
    stones: RefCell<HashMap<String, HtmlCanvasElement>>,
}

impl OraclePainter {
    pub fn new(context: CanvasRenderingContext2d, scale: f64) -> Self {
        let marks = FumePainter::with_pictures(context.clone(), &[FumeBowl::PICTURE]);
        let photo = HtmlImageElement::new().ok().inspect(|image| image.set_src(Limestone::PHOTO));
        Self { context, marks, photo, scale, stones: RefCell::new(HashMap::new()) }
    }

    pub fn paint(&self, sketch: &OracleSketch, width: f64, height: f64) {
        self.marks.clear(sketch.background, width, height);
        self.marks.marks(&sketch.under, width);
        let keys: Vec<String> = sketch.stones.iter().map(|stone| self.key(stone)).collect();
        self.stones.borrow_mut().retain(|key, _| keys.contains(key));
        for (stone, key) in sketch.stones.iter().zip(&keys) {
            self.stone(stone, key);
        }
        self.context.set_global_alpha(1.0);
        self.marks.marks(&sketch.front, width);
        self.marks.marks(&sketch.over, width);
    }

    fn key(&self, stone: &StoneSketch) -> String {
        format!("{}:{:.3}", stone.cache_key(self.scale), stone.shade)
    }

    fn stone(&self, sketch: &StoneSketch, key: &str) {
        let Some(photo) = self.photo.as_ref().filter(|photo| photo.complete() && photo.natural_width() > 0) else {
            return;
        };
        if !self.stones.borrow().contains_key(key) {
            let Some(cut) = StoneCutter::cut(sketch, photo, self.scale) else {
                return;
            };
            self.stones.borrow_mut().insert(key.to_owned(), cut);
        }
        let stones = self.stones.borrow();
        let Some(canvas) = stones.get(key) else {
            return;
        };
        let face = sketch.face(self.scale);
        let (left, top) = (face.origin().0 + sketch.offset.0, face.origin().1 + sketch.offset.1);
        let (width, height) = (f64::from(canvas.width()) / self.scale, f64::from(canvas.height()) / self.scale);
        self.context.set_global_composite_operation("source-over").ok();
        self.context.set_global_alpha(sketch.alpha.clamp(0.0, 1.0));
        self.context.draw_image_with_html_canvas_element_and_dw_and_dh(canvas, left, top, width, height).ok();
    }
}
