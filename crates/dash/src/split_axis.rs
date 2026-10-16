/// The direction a split lays its children out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Row,
    Column,
}

impl SplitAxis {
    #[must_use]
    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Column => "column",
        }
    }

    /// The component of a horizontal/vertical pair that runs along this axis.
    #[must_use]
    pub const fn along(self, horizontal: f64, vertical: f64) -> f64 {
        match self {
            Self::Row => horizontal,
            Self::Column => vertical,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SplitAxis;

    #[test]
    fn each_axis_picks_its_own_component() {
        assert!((SplitAxis::Row.along(3.0, 7.0) - 3.0).abs() < 1e-9);
        assert!((SplitAxis::Column.along(3.0, 7.0) - 7.0).abs() < 1e-9);
    }
}
