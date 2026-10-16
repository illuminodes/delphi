#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarveAlign {
    Start,
    Centre,
    End,
}

impl CarveAlign {
    #[must_use]
    pub const fn css(self) -> &'static str {
        match self {
            Self::Start => "left",
            Self::Centre => "center",
            Self::End => "right",
        }
    }

    #[must_use]
    pub fn left_of(self, anchor: f64, width: f64) -> f64 {
        match self {
            Self::Start => anchor,
            Self::Centre => anchor - width / 2.0,
            Self::End => anchor - width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CarveAlign;

    #[test]
    fn the_anchor_sits_at_the_start_middle_or_end_of_the_text() {
        assert!((CarveAlign::Start.left_of(100.0, 40.0) - 100.0).abs() < 1e-9);
        assert!((CarveAlign::Centre.left_of(100.0, 40.0) - 80.0).abs() < 1e-9);
        assert!((CarveAlign::End.left_of(100.0, 40.0) - 60.0).abs() < 1e-9);
    }
}
