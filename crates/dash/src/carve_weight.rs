#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarveWeight {
    Hair,
    Rule,
    Bold,
}

impl CarveWeight {
    #[must_use]
    pub const fn width(self) -> f64 {
        match self {
            Self::Hair => 1.6,
            Self::Rule => 1.8,
            Self::Bold => 2.6,
        }
    }

    #[must_use]
    pub const fn depth(self) -> f64 {
        match self {
            Self::Hair => 0.5,
            Self::Rule => 0.6,
            Self::Bold => 0.9,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CarveWeight;

    #[test]
    fn heavier_lines_are_wider_and_deeper() {
        let weights = [CarveWeight::Hair, CarveWeight::Rule, CarveWeight::Bold];

        assert!(weights.windows(2).all(|pair| pair[0].width() < pair[1].width() && pair[0].depth() < pair[1].depth()));
    }
}
