use std::error::Error;

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use mantle_dash::{CarveAlign, CarveFont, CarveMask, CarveText};

pub struct GlyphLetterer {
    faces: [FontVec; 3],
    scale: f64,
}

impl GlyphLetterer {
    const FACES: [&str; 3] = ["MarcellusSC-Regular.ttf", "AlegreyaSans-Regular.ttf", "IBMPlexMono-Regular.ttf"];
    const FOLDER: &str = "/tmp/delphi-marble/fonts";

    pub fn new(scale: f64) -> Result<Self, Box<dyn Error>> {
        let load = |name: &str| -> Result<FontVec, Box<dyn Error>> { Ok(FontVec::try_from_vec(std::fs::read(format!("{}/{name}", Self::FOLDER))?)?) };
        Ok(Self { faces: [load(Self::FACES[0])?, load(Self::FACES[1])?, load(Self::FACES[2])?], scale })
    }

    fn face(&self, font: CarveFont) -> &FontVec {
        match font {
            CarveFont::Display => &self.faces[0],
            CarveFont::Text => &self.faces[1],
            CarveFont::Figures => &self.faces[2],
        }
    }

    pub fn letter(&self, mask: &mut CarveMask, text: &CarveText) {
        let font = self.face(text.font);
        let size = (text.size * self.scale) as f32;
        let scaled = font.as_scaled(PxScale::from(size));
        let tracking = (text.tracking * text.size * self.scale) as f32;
        let advance: f32 = text.text.chars().map(|letter| scaled.h_advance(font.glyph_id(letter)) + tracking).sum();
        let anchor = (text.x * self.scale) as f32;
        let mut pen = match text.align {
            CarveAlign::Start => anchor,
            CarveAlign::Centre => anchor - advance / 2.0,
            CarveAlign::End => anchor - advance,
        };
        let baseline = (text.y * self.scale) as f32;
        for letter in text.text.chars() {
            let id = font.glyph_id(letter);
            if let Some(outline) = font.outline_glyph(id.with_scale_and_position(size, point(pen, baseline))) {
                let bounds = outline.px_bounds();
                outline.draw(|x, y, cover| {
                    let (column, row) = (bounds.min.x as i64 + i64::from(x), bounds.min.y as i64 + i64::from(y));
                    if column >= 0 && row >= 0 {
                        mask.press(column as usize, row as usize, f64::from(cover), text.depth, text.fill, text.cut);
                    }
                });
            }
            pen += scaled.h_advance(id) + tracking;
        }
    }
}
