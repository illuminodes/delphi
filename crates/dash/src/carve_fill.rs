use crate::CarveCut;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarveFill {
    Plain,
    Gilt,
}

impl CarveFill {
    #[must_use]
    pub fn mask_colour(self, depth: f64, cut: CarveCut) -> String {
        let level = (depth.clamp(0.0, 1.0) * 255.0).round() as u8;
        let gilt = match self {
            Self::Plain => 0,
            Self::Gilt => level,
        };
        format!("rgb({level}, {gilt}, {})", cut.mask_level(level))
    }
}

#[cfg(test)]
mod tests {
    use super::CarveFill;
    use crate::CarveCut;

    #[test]
    fn red_carries_the_depth_green_the_gilt_and_blue_the_incision() {
        assert_eq!(CarveFill::Plain.mask_colour(1.0, CarveCut::Relief), "rgb(255, 0, 0)");
        assert_eq!(CarveFill::Gilt.mask_colour(0.5, CarveCut::Relief), "rgb(128, 128, 0)");
        assert_eq!(CarveFill::Plain.mask_colour(4.0, CarveCut::Incised), "rgb(255, 0, 255)");
    }
}
