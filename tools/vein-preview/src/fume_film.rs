use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use image::{imageops::FilterType, RgbaImage};
use mantle_dash::{FumeBowl, FumeMark, FumeScene, FumeSketch, VesselMask};

pub struct FumeFilm {
    width: u32,
    height: u32,
    seconds: Vec<f64>,
    harvest_at: Option<f64>,
    pictures: HashMap<&'static str, RgbaImage>,
    shape: RgbaImage,
}

impl FumeFilm {
    const STEP: f64 = 1.0 / 60.0;
    const SEED: u64 = 7;

    const PUBLIC: &'static str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/dash/public/");

    pub fn new(width: u32, height: u32, times: &[String]) -> Result<Self, Box<dyn Error>> {
        let pictures = [FumeSketch::VESSEL, FumeBowl::PICTURE]
            .into_iter()
            .map(|source| Ok((source, image::open(format!("{}{}", Self::PUBLIC, source))?.to_rgba8())))
            .collect::<Result<HashMap<_, _>, Box<dyn Error>>>()?;
        let shape = image::open(format!("{}{}", Self::PUBLIC, FumeSketch::VESSEL_MASK))?.to_rgba8();
        let harvest_at = times.iter().find_map(|time| time.strip_prefix('h')).map(str::parse).transpose()?;
        let seconds = times.iter().filter(|time| !time.starts_with('h')).map(|time| time.parse::<f64>()).collect::<Result<Vec<_>, _>>()?;
        Ok(Self { width, height, seconds, harvest_at, pictures, shape })
    }

    pub fn save(&self, stem: &Path) -> Result<(), Box<dyn Error>> {
        let mut scene = FumeScene::new(f64::from(self.width), f64::from(self.height), Self::SEED);
        let mut frames = Vec::new();
        let mut harvest_at = self.harvest_at;
        for target in &self.seconds {
            while scene.now() + Self::STEP * 0.5 < *target {
                if harvest_at.is_some_and(|at| scene.now() >= at) {
                    harvest_at = None;
                    println!("harvest at t={:.2}s: {}", scene.now(), scene.harvest());
                }
                scene.advance(Self::STEP);
            }
            frames.push(self.frame(&scene)?);
            let counts: Vec<String> = scene.jars().iter().map(|jar| format!("{} {}", jar.fate.label(), jar.collected())).collect();
            let fullness: Vec<String> = scene.jars().iter().map(|jar| format!("{:.2}", jar.fullness())).collect();
            println!(
                "t={:>5.1}s  in flight {:>3}  {}  turned away {}  full [{}]  bowl {:.2} glow {:.2}",
                scene.now(),
                scene.in_flight(),
                counts.join("  "),
                scene.scattered(),
                fullness.join(" "),
                scene.brew().charge(),
                scene.brew().glow()
            );
        }
        let mut sheet = RgbaImage::new(self.width, self.height * frames.len() as u32);
        for (index, frame) in frames.iter().enumerate() {
            image::imageops::replace(&mut sheet, frame, 0, i64::from(self.height) * index as i64);
        }
        sheet.save(stem)?;
        println!("wrote {}", stem.display());
        Ok(())
    }

    fn frame(&self, scene: &FumeScene) -> Result<RgbaImage, Box<dyn Error>> {
        let sketch = FumeSketch::of(scene);
        let base = sketch.background.channels().map(f64::from);
        let mut canvas = vec![base; (self.width * self.height) as usize];
        for mark in &sketch.marks {
            if let FumeMark::Picture { source, left, top, width, height, alpha } = mark {
                self.stamp(&mut canvas, source, (*left, *top, *width, *height), *alpha);
                continue;
            }
            let mask = if let FumeMark::Gas { mask, .. } = mark { Some(mask) } else { None };
            for row in 0..self.height {
                for column in 0..self.width {
                    let at = (row * self.width + column) as usize;
                    let (x, y) = (f64::from(column) + 0.5, f64::from(row) + 0.5);
                    let painted = mark.paint(canvas[at], x, y);
                    let keep = mask.map_or(1.0, |mask| self.inside(mask, x, y));
                    canvas[at] = [0, 1, 2].map(|channel| canvas[at][channel] + (painted[channel] - canvas[at][channel]) * keep);
                }
            }
        }
        let pixels = canvas.into_iter().flat_map(|colour| {
            let [red, green, blue] = colour.map(|channel| channel.round().clamp(0.0, 255.0) as u8);
            [red, green, blue, 255]
        });
        RgbaImage::from_raw(self.width, self.height, pixels.collect()).ok_or_else(|| "bad frame".into())
    }

    fn inside(&self, mask: &VesselMask, x: f64, y: f64) -> f64 {
        let Some((across, down)) = mask.place(x, y) else {
            return 0.0;
        };
        let (column, row) = ((across * f64::from(self.shape.width())) as u32, (down * f64::from(self.shape.height())) as u32);
        f64::from(self.shape.get_pixel(column.min(self.shape.width() - 1), row.min(self.shape.height() - 1)).0[3]) / 255.0
    }

    fn stamp(&self, canvas: &mut [[f64; 3]], source: &str, (left, top, width, height): (f64, f64, f64, f64), alpha: f64) {
        let Some(picture) = self.pictures.get(source) else {
            return;
        };
        let (wide, tall) = (width.round().max(1.0) as u32, height.round().max(1.0) as u32);
        let sized = image::imageops::resize(picture, wide, tall, FilterType::Triangle);
        let (left, top) = (left.round() as i64, top.round() as i64);
        for (x, y, pixel) in sized.enumerate_pixels() {
            let (column, row) = (left + i64::from(x), top + i64::from(y));
            if column < 0 || row < 0 || column >= i64::from(self.width) || row >= i64::from(self.height) {
                continue;
            }
            let cover = f64::from(pixel.0[3]) / 255.0 * alpha;
            let at = (row as u32 * self.width + column as u32) as usize;
            canvas[at] = [0, 1, 2].map(|channel| canvas[at][channel] + (f64::from(pixel.0[channel]) - canvas[at][channel]) * cover);
        }
    }
}
