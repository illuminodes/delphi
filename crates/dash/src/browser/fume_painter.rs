use std::f64::consts::TAU;

use web_sys::{CanvasRenderingContext2d, HtmlImageElement};

use super::gas_stamp::GasStamp;
use crate::{FumeBowl, FumeMark, FumePolygon, FumeSketch, GasCloud, HexColor, VesselMask};

pub struct FumePainter {
    context: CanvasRenderingContext2d,
    pictures: Vec<(&'static str, HtmlImageElement)>,
    stamp: Option<GasStamp>,
}

impl FumePainter {
    pub fn new(context: CanvasRenderingContext2d) -> Self {
        Self::with_pictures(context, &[FumeSketch::VESSEL, FumeSketch::VESSEL_MASK, FumeBowl::PICTURE])
    }

    pub fn with_pictures(context: CanvasRenderingContext2d, sources: &[&'static str]) -> Self {
        let pictures = sources
            .iter()
            .copied()
            .filter_map(|source| {
                let image = HtmlImageElement::new().ok()?;
                image.set_src(source);
                Some((source, image))
            })
            .collect();
        Self { context, pictures, stamp: GasStamp::new() }
    }

    fn loaded(&self, source: &str) -> Option<&HtmlImageElement> {
        self.pictures.iter().find(|(name, image)| *name == source && image.complete() && image.natural_width() > 0).map(|(_, image)| image)
    }

    fn picture(&self, source: &str, left: f64, top: f64, width: f64, height: f64, alpha: f64) {
        let Some(image) = self.loaded(source) else {
            return;
        };
        self.context.set_global_composite_operation("source-over").ok();
        self.context.set_global_alpha(alpha);
        self.context.draw_image_with_html_image_element_and_dw_and_dh(image, left, top, width, height).ok();
    }

    pub fn paint(&self, sketch: &FumeSketch, width: f64, height: f64) {
        self.clear(sketch.background, width, height);
        self.marks(&sketch.marks, width);
    }

    pub fn clear(&self, background: HexColor, width: f64, height: f64) {
        self.context.set_global_composite_operation("source-over").ok();
        self.context.set_global_alpha(1.0);
        self.context.set_fill_style_str(&background.hex());
        self.context.fill_rect(0.0, 0.0, width, height);
    }

    pub fn marks(&self, marks: &[FumeMark], width: f64) {
        for mark in marks {
            self.mark(mark, width);
        }
        self.context.set_global_composite_operation("source-over").ok();
        self.context.set_global_alpha(1.0);
    }

    fn mark(&self, mark: &FumeMark, width: f64) {
        match mark {
            FumeMark::Glow { x, y, radius, colour, alpha } => self.glow(*x, *y, *radius, *colour, *alpha),
            FumeMark::Fill { shape, colour, alpha, below } => {
                self.context.save();
                if let Some(line) = below {
                    self.context.begin_path();
                    self.context.rect(0.0, *line, width, shape.bottom() - line + 1.0);
                    self.context.clip();
                }
                self.context.set_global_composite_operation("source-over").ok();
                self.context.set_global_alpha(*alpha);
                self.context.set_fill_style_str(&colour.hex());
                self.trace(shape);
                self.context.fill();
                self.context.restore();
            }
            FumeMark::Picture { source, left, top, width, height, alpha } => self.picture(source, *left, *top, *width, *height, *alpha),
            FumeMark::Gas { cloud, mask } => self.gas(cloud, mask),
        }
    }

    fn gas(&self, cloud: &GasCloud, mask: &VesselMask) {
        let (Some(stamp), Some(shape)) = (&self.stamp, self.loaded(mask.source)) else {
            return;
        };
        stamp.draw(cloud, shape, mask, &self.context);
    }

    fn glow(&self, x: f64, y: f64, radius: f64, colour: HexColor, alpha: f64) {
        let Ok(gradient) = self.context.create_radial_gradient(x, y, 0.0, x, y, radius) else {
            return;
        };
        let [red, green, blue] = colour.channels();
        for (stop, strength) in [(0.0, 1.0), (0.35, 0.66), (0.7, 0.18), (1.0, 0.0)] {
            gradient.add_color_stop(stop, &format!("rgba({red},{green},{blue},{strength})")).ok();
        }
        self.context.set_global_composite_operation("lighter").ok();
        self.context.set_global_alpha(alpha.clamp(0.0, 1.0));
        self.context.set_fill_style_canvas_gradient(&gradient);
        self.context.begin_path();
        self.context.arc(x, y, radius, 0.0, TAU).ok();
        self.context.fill();
    }

    fn trace(&self, shape: &FumePolygon) {
        self.context.begin_path();
        for (index, (x, y)) in shape.points().iter().enumerate() {
            if index == 0 {
                self.context.move_to(*x, *y);
            } else {
                self.context.line_to(*x, *y);
            }
        }
        self.context.close_path();
    }
}
