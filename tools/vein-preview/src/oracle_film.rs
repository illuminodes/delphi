use std::error::Error;
use std::path::Path;

use image::{imageops::FilterType, RgbaImage};
use delphi_dash::{CarveMask, Carving, FumeBowl, FumeMark, Limestone, OracleScene, OracleSketch, StoneSketch};

use crate::glyph_letterer::GlyphLetterer;

pub struct OracleFilm {
    width: u32,
    height: u32,
    seconds: Vec<f64>,
    photo: RgbaImage,
    tripod: RgbaImage,
}

impl OracleFilm {
    const STEP: f64 = 1.0 / 30.0;
    const SEED: u64 = 7;
    const PUBLIC: &'static str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/dash/public/");

    pub fn new(width: u32, height: u32, times: &[String]) -> Result<Self, Box<dyn Error>> {
        let open = |source: &str| -> Result<RgbaImage, Box<dyn Error>> { Ok(image::open(format!("{}{source}", Self::PUBLIC))?.to_rgba8()) };
        let seconds = times.iter().map(|time| time.parse::<f64>()).collect::<Result<Vec<_>, _>>()?;
        Ok(Self { width, height, seconds, photo: open(Limestone::PHOTO)?, tripod: open(FumeBowl::PICTURE)? })
    }

    pub fn save(&self, output: &Path) -> Result<(), Box<dyn Error>> {
        let mut scene = OracleScene::simulated(f64::from(self.width), f64::from(self.height), Self::SEED);
        let letterer = GlyphLetterer::new(1.0)?;
        let mut frames = Vec::new();
        for target in &self.seconds {
            while scene.now() + Self::STEP * 0.5 < *target {
                if scene.advance(Self::STEP) {
                    println!("block {} arrived at t={:.2}s", scene.wall().newest().map_or(0, |block| block.height), scene.now());
                }
            }
            let arrival = scene.arrival();
            println!(
                "t={:>5.1}s  stones {}  strike {:.2} cross {:.2} settle {:.2} carve {:.2} beam {:.2}  waited {:.0}s",
                scene.now(),
                scene.wall().blocks().len(),
                arrival.strike(),
                arrival.cross(),
                arrival.settle(),
                arrival.carve(),
                arrival.beam(),
                scene.waited()
            );
            frames.push(self.frame(&OracleSketch::of(&scene), &letterer)?);
        }
        let mut sheet = RgbaImage::new(self.width, self.height * frames.len() as u32);
        for (index, frame) in frames.iter().enumerate() {
            image::imageops::replace(&mut sheet, frame, 0, i64::from(self.height) * index as i64);
        }
        sheet.save(output)?;
        println!("wrote {}", output.display());
        Ok(())
    }

    fn frame(&self, sketch: &OracleSketch, letterer: &GlyphLetterer) -> Result<RgbaImage, Box<dyn Error>> {
        let base = sketch.background.channels().map(f64::from);
        let mut canvas = vec![base; (self.width * self.height) as usize];
        for mark in &sketch.under {
            self.mark(&mut canvas, mark);
        }
        for stone in &sketch.stones {
            self.stone(&mut canvas, stone, letterer);
        }
        for mark in &sketch.front {
            self.mark(&mut canvas, mark);
        }
        for mark in &sketch.over {
            self.mark(&mut canvas, mark);
        }
        let pixels = canvas.into_iter().flat_map(|colour| {
            let [red, green, blue] = colour.map(|channel| channel.round().clamp(0.0, 255.0) as u8);
            [red, green, blue, 255]
        });
        RgbaImage::from_raw(self.width, self.height, pixels.collect()).ok_or_else(|| "bad frame".into())
    }

    fn mark(&self, canvas: &mut [[f64; 3]], mark: &FumeMark) {
        if let FumeMark::Picture { left, top, width, height, alpha, .. } = mark {
            let sized = image::imageops::resize(&self.tripod, width.round().max(1.0) as u32, height.round().max(1.0) as u32, FilterType::Triangle);
            self.blend(canvas, &sized, (left.round() as i64, top.round() as i64), *alpha, 0.0);
            return;
        }
        for row in 0..self.height {
            for column in 0..self.width {
                let at = (row * self.width + column) as usize;
                canvas[at] = mark.paint(canvas[at], f64::from(column) + 0.5, f64::from(row) + 0.5);
            }
        }
    }

    fn stone(&self, canvas: &mut [[f64; 3]], sketch: &StoneSketch, letterer: &GlyphLetterer) {
        let face = sketch.face(1.0);
        let (width, height) = face.pixel_size();
        let photo = (f64::from(self.photo.width()), f64::from(self.photo.height()));
        let crop = sketch.patch.crop(photo, (width as f64, height as f64));
        let cut = image::imageops::crop_imm(&self.photo, crop.x as u32, crop.y as u32, crop.width.max(1.0) as u32, crop.height.max(1.0) as u32).to_image();
        let mut sized = image::imageops::resize(&cut, width as u32, height as u32, FilterType::Lanczos3);
        if sketch.patch.mirrored {
            image::imageops::flip_horizontal_in_place(&mut sized);
        }
        if sketch.patch.flipped {
            image::imageops::flip_vertical_in_place(&mut sized);
        }
        let mut pixels = sized.into_raw();
        let carvings = face.local(&sketch.carvings);
        let mask = (!carvings.is_empty()).then(|| {
            let mut mask = CarveMask::new(width, height, face.scale());
            for carving in &carvings {
                if let Carving::Text(text) = carving {
                    letterer.letter(&mut mask, text);
                }
            }
            mask.thicken_relief(Limestone::ETCH.thicken);
            mask
        });
        face.dress(&mut pixels, mask.as_ref(), Limestone::ETCH);
        let Some(dressed) = RgbaImage::from_raw(width as u32, height as u32, pixels) else {
            return;
        };
        let origin = face.origin();
        let at = ((origin.0 + sketch.offset.0).round() as i64, (origin.1 + sketch.offset.1).round() as i64);
        self.blend(canvas, &dressed, at, sketch.alpha, sketch.shade);
    }

    fn blend(&self, canvas: &mut [[f64; 3]], picture: &RgbaImage, (left, top): (i64, i64), alpha: f64, shade: f64) {
        for (x, y, pixel) in picture.enumerate_pixels() {
            let (column, row) = (left + i64::from(x), top + i64::from(y));
            if column < 0 || row < 0 || column >= i64::from(self.width) || row >= i64::from(self.height) {
                continue;
            }
            let cover = f64::from(pixel.0[3]) / 255.0 * alpha;
            let at = (row as u32 * self.width + column as u32) as usize;
            canvas[at] = [0, 1, 2].map(|channel| {
                let ink = f64::from(pixel.0[channel]) * (1.0 - shade);
                canvas[at][channel] + (ink - canvas[at][channel]) * cover
            });
        }
    }
}
