use crate::{CarveCavity, CarveMask, CarveRelief, EtchRecipe, SlabFinish};

#[derive(Debug, Clone, PartialEq)]
pub struct SlabSculpt {
    width: f64,
    height: f64,
    relief: CarveRelief,
    albedo: Vec<u8>,
    normals: Vec<u8>,
    surface: Vec<u8>,
}

impl SlabSculpt {
    const SLOPE: f64 = 1.8;
    const FLOOR: f64 = 14.0;

    #[must_use]
    pub fn new(mask: &CarveMask, photo: &[u8], recipe: EtchRecipe) -> Self {
        let relief = CarveRelief::new(mask, Self::SLOPE, Self::FLOOR);
        let finish = SlabFinish::new(recipe);
        let mut albedo = finish.albedo(photo, mask, &relief);
        CarveCavity::new(&relief, mask.scale()).shade(&mut albedo);
        Self {
            width: mask.width() as f64 / mask.scale(),
            height: mask.height() as f64 / mask.scale(),
            normals: relief.normals(),
            albedo,
            surface: finish.surface(mask, &relief),
            relief,
        }
    }

    #[must_use]
    pub const fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    #[must_use]
    pub const fn relief(&self) -> &CarveRelief {
        &self.relief
    }

    #[must_use]
    pub fn albedo(&self) -> &[u8] {
        &self.albedo
    }

    #[must_use]
    pub fn normals(&self) -> &[u8] {
        &self.normals
    }

    #[must_use]
    pub fn surface(&self) -> &[u8] {
        &self.surface
    }
}

#[cfg(test)]
mod tests {
    use super::SlabSculpt;
    use crate::{CarveGroove, CarveMask, MarbleKind};

    #[test]
    fn every_map_covers_the_whole_slab_in_css_pixels() {
        let mut mask = CarveMask::new(80, 40, 2.0);
        mask.stamp_groove(&CarveGroove::line((5.0, 10.0), (35.0, 10.0), 4.0));
        let sculpt = SlabSculpt::new(&mask, &[200; 80 * 40 * 4], MarbleKind::Parian.etch());

        assert_eq!(sculpt.size(), (40.0, 20.0));
        assert_eq!(sculpt.relief().depths().len(), 80 * 40);
        for map in [sculpt.albedo(), sculpt.normals(), sculpt.surface()] {
            assert_eq!(map.len(), 80 * 40 * 4);
        }
        assert!(sculpt.relief().at(40, 20) > 1.0);
    }
}
