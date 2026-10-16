#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarveBox {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl CarveBox {
    const SLACK: f64 = 1e-6;

    #[must_use]
    pub const fn new(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self { left, top, right, bottom }
    }

    #[must_use]
    pub fn width(self) -> f64 {
        self.right - self.left
    }

    #[must_use]
    pub fn height(self) -> f64 {
        self.bottom - self.top
    }

    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.left < other.right && other.left < self.right && self.top < other.bottom && other.top < self.bottom
    }

    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        other.left >= self.left - Self::SLACK
            && other.right <= self.right + Self::SLACK
            && other.top >= self.top - Self::SLACK
            && other.bottom <= self.bottom + Self::SLACK
    }

    #[must_use]
    pub fn grow(self, by: f64) -> Self {
        Self::new(self.left - by, self.top - by, self.right + by, self.bottom + by)
    }

    #[must_use]
    pub fn columns(self, count: usize, gap: f64) -> Vec<Self> {
        let count = count.max(1);
        let width = (self.width() - gap * (count - 1) as f64) / count as f64;
        (0..count)
            .map(|index| {
                let left = self.left + index as f64 * (width + gap);
                Self::new(left, self.top, left + width, self.bottom)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::CarveBox;

    #[test]
    fn columns_tile_the_box_with_the_gap_between_them() {
        let columns = CarveBox::new(0.0, 0.0, 340.0, 50.0).columns(4, 20.0);

        assert_eq!(columns.len(), 4);
        assert!((columns[0].left).abs() < 1e-9);
        assert!((columns[3].right - 340.0).abs() < 1e-9);
        for pair in columns.windows(2) {
            assert!((pair[1].left - pair[0].right - 20.0).abs() < 1e-9);
        }
    }

    #[test]
    fn boxes_that_only_touch_do_not_overlap() {
        let one = CarveBox::new(0.0, 0.0, 10.0, 10.0);

        assert!(!one.overlaps(CarveBox::new(10.0, 0.0, 20.0, 10.0)));
        assert!(one.overlaps(CarveBox::new(9.0, 9.0, 20.0, 20.0)));
        assert!(CarveBox::new(9.0, 9.0, 20.0, 20.0).overlaps(one));
    }

    #[test]
    fn a_grown_box_contains_the_original() {
        let one = CarveBox::new(5.0, 5.0, 10.0, 10.0);

        assert!(one.grow(2.0).contains(one));
        assert!(!one.contains(one.grow(2.0)));
    }
}
