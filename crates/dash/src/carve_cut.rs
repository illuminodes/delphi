#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarveCut {
    Relief,
    Incised,
}

impl CarveCut {
    const RELIEF_FROM: f64 = 18.0;

    #[must_use]
    pub fn for_size(size: f64) -> Self {
        if size >= Self::RELIEF_FROM { Self::Relief } else { Self::Incised }
    }

    #[must_use]
    pub const fn mask_level(self, level: u8) -> u8 {
        match self {
            Self::Relief => 0,
            Self::Incised => level,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CarveCut;

    #[test]
    fn large_letters_are_cut_in_relief_and_small_ones_incised() {
        assert_eq!(CarveCut::for_size(46.0), CarveCut::Relief);
        assert_eq!(CarveCut::for_size(18.0), CarveCut::Relief);
        assert_eq!(CarveCut::for_size(11.0), CarveCut::Incised);
    }

    #[test]
    fn only_incised_cuts_mark_the_mask() {
        assert_eq!(CarveCut::Relief.mask_level(200), 0);
        assert_eq!(CarveCut::Incised.mask_level(200), 200);
    }
}
