use crate::{CarveBox, CarveFill, CarveGroove, CarveText};

#[derive(Debug, Clone, PartialEq)]
pub enum Carving {
    Text(CarveText),
    Groove(CarveGroove),
}

impl Carving {
    #[must_use]
    pub fn bounds(&self) -> CarveBox {
        match self {
            Self::Text(text) => text.bounds(),
            Self::Groove(groove) => groove.bounds(),
        }
    }

    #[must_use]
    pub fn shifted(&self, dx: f64, dy: f64) -> Self {
        match self {
            Self::Text(text) => Self::Text(CarveText { x: text.x + dx, y: text.y + dy, ..text.clone() }),
            Self::Groove(groove) => Self::Groove(CarveGroove { points: groove.points.iter().map(|(x, y)| (x + dx, y + dy)).collect(), ..groove.clone() }),
        }
    }

    #[must_use]
    pub const fn depth(&self) -> f64 {
        match self {
            Self::Text(text) => text.depth,
            Self::Groove(groove) => groove.depth,
        }
    }

    #[must_use]
    pub const fn fill(&self) -> CarveFill {
        match self {
            Self::Text(text) => text.fill,
            Self::Groove(groove) => groove.fill,
        }
    }
}

impl From<CarveText> for Carving {
    fn from(text: CarveText) -> Self {
        Self::Text(text)
    }
}

impl From<CarveGroove> for Carving {
    fn from(groove: CarveGroove) -> Self {
        Self::Groove(groove)
    }
}

#[cfg(test)]
mod tests {
    use super::Carving;
    use crate::{CarveFill, CarveFont, CarveGroove, CarveText};

    #[test]
    fn a_carving_reports_the_depth_fill_and_bounds_of_its_cut() {
        let text: Carving = CarveText::new("Δ", CarveFont::Display, 10.0, 0.0, 10.0).depth(0.5).gilt().into();
        let groove: Carving = CarveGroove::line((0.0, 0.0), (10.0, 0.0), 2.0).into();

        assert!((text.depth() - 0.5).abs() < 1e-9);
        assert_eq!(text.fill(), CarveFill::Gilt);
        assert!((groove.bounds().width() - 12.0).abs() < 1e-9);
    }

    #[test]
    fn a_shifted_carving_moves_its_bounds_and_keeps_its_cut() {
        let text: Carving = CarveText::new("Δ", CarveFont::Display, 10.0, 0.0, 10.0).gilt().into();
        let groove: Carving = CarveGroove::line((0.0, 0.0), (10.0, 0.0), 2.0).into();

        for carving in [text, groove] {
            let moved = carving.shifted(5.0, -3.0);
            assert!((moved.bounds().left - carving.bounds().left - 5.0).abs() < 1e-9);
            assert!((moved.bounds().top - carving.bounds().top + 3.0).abs() < 1e-9);
            assert_eq!(moved.fill(), carving.fill());
        }
    }
}
