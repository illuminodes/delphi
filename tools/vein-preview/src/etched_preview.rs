use std::error::Error;
use std::path::Path;

use image::imageops::FilterType;
use image::RgbaImage;
use mantle_dash::{CarveMask, Carving, MarbleKind, PhotoCrop, StoneEtch, WireframePlan};

use crate::glyph_letterer::GlyphLetterer;
use crate::grain_report::GrainReport;

pub struct EtchedPreview {
    kind: MarbleKind,
    seed: u64,
    width: usize,
    height: usize,
}

impl EtchedPreview {
    const SCALE: f64 = 2.0;

    pub const fn new(kind: MarbleKind, seed: u64, width: usize, height: usize) -> Self {
        Self { kind, seed, width, height }
    }

    pub fn save(&self, output: &Path) -> Result<(), Box<dyn Error>> {
        let (width, height) = self.pixel_size();
        let mask = self.mask()?;
        let stone = self.stone()?;
        let pixels = self.etched(&stone, &mask);
        GrainReport::new(width, height).print(&mask, &stone, &pixels);
        if let Ok(other) = std::env::var("VEIN_COMPARE") {
            GrainReport::new(width, height).print(&mask, &stone, &image::open(other)?.to_rgba8().into_raw());
        }
        RgbaImage::from_raw(width as u32, height as u32, pixels).ok_or("bad render")?.save(output)?;
        Ok(())
    }

    pub fn etched(&self, stone: &[u8], mask: &CarveMask) -> Vec<u8> {
        let mut pixels = stone.to_vec();
        StoneEtch::new(self.kind.etch(), self.seed).apply(&mut pixels, mask);
        pixels
    }

    pub const fn kind(&self) -> MarbleKind {
        self.kind
    }

    pub fn mask(&self) -> Result<CarveMask, Box<dyn Error>> {
        let plan = WireframePlan::new(self.width as f64, self.height as f64, self.kind, self.seed);
        let (width, height) = self.pixel_size();
        let letterer = GlyphLetterer::new(Self::SCALE)?;
        let mut mask = CarveMask::new(width, height, Self::SCALE);
        for carving in plan.carvings() {
            if let Carving::Text(text) = carving {
                letterer.letter(&mut mask, text);
            }
        }
        mask.thicken_relief(self.kind.etch().thicken);
        for carving in plan.carvings() {
            if let Carving::Groove(groove) = carving {
                mask.stamp_groove(groove);
            }
        }
        Ok(mask)
    }

    fn pixel_size(&self) -> (usize, usize) {
        ((self.width as f64 * Self::SCALE) as usize, (self.height as f64 * Self::SCALE) as usize)
    }

    pub fn stone(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        let (width, height) = self.pixel_size();
        let photo = self.kind.photo().ok_or("this stone has no photo")?;
        let image = image::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/dash/public").join(photo))?.to_rgba8();
        let crop = PhotoCrop::cover((f64::from(image.width()), f64::from(image.height())), (width as f64, height as f64));
        let cropped = image::imageops::crop_imm(&image, crop.x as u32, crop.y as u32, crop.width as u32, crop.height as u32).to_image();
        Ok(image::imageops::resize(&cropped, width as u32, height as u32, FilterType::Lanczos3).into_raw())
    }
}
