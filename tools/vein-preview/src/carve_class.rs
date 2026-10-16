use mantle_dash::CarveMask;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarveClass {
    Relief,
    Gilt,
    Line,
    Incised,
}

impl CarveClass {
    pub const ALL: [Self; 4] = [Self::Relief, Self::Gilt, Self::Line, Self::Incised];
    const MARKED: f64 = 0.5;

    pub fn of(mask: &CarveMask, column: usize, row: usize) -> Self {
        if mask.gilt_at(column, row) >= Self::MARKED {
            Self::Gilt
        } else if mask.grooved_at(column, row) >= Self::MARKED {
            Self::Line
        } else if mask.incised_at(column, row) >= Self::MARKED {
            Self::Incised
        } else {
            Self::Relief
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Relief => "relief",
            Self::Gilt => "gilt",
            Self::Line => "line",
            Self::Incised => "incised",
        }
    }
}
