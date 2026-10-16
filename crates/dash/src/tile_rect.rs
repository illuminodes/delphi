use crate::SplitAxis;

/// A rectangle in workspace fractions: (0, 0) is the top-left corner, (1, 1) the bottom-right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl TileRect {
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    #[must_use]
    pub fn split(self, axis: SplitAxis, ratio: f64) -> (Self, Self) {
        match axis {
            SplitAxis::Row => {
                let first = self.width * ratio;
                (
                    Self { width: first, ..self },
                    Self {
                        x: self.x + first,
                        width: self.width - first,
                        ..self
                    },
                )
            }
            SplitAxis::Column => {
                let first = self.height * ratio;
                (
                    Self { height: first, ..self },
                    Self {
                        y: self.y + first,
                        height: self.height - first,
                        ..self
                    },
                )
            }
        }
    }

    #[must_use]
    pub const fn along(self, axis: SplitAxis) -> f64 {
        match axis {
            SplitAxis::Row => self.width,
            SplitAxis::Column => self.height,
        }
    }

    #[must_use]
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    /// Where `(x, y)` sits inside the rectangle, as fractions of its width and height.
    #[must_use]
    pub fn fraction(self, x: f64, y: f64) -> Option<(f64, f64)> {
        (self.width > 0.0 && self.height > 0.0).then(|| ((x - self.x) / self.width, (y - self.y) / self.height))
    }

    #[must_use]
    pub fn area(self) -> f64 {
        self.width * self.height
    }

    /// The axis along which the rectangle is longer on a workspace `aspect` (width / height) wide.
    #[must_use]
    pub fn longer_axis(self, aspect: f64) -> SplitAxis {
        if self.width * aspect >= self.height {
            SplitAxis::Row
        } else {
            SplitAxis::Column
        }
    }

    #[must_use]
    pub fn style(self) -> String {
        format!(
            "left:{:.4}%;top:{:.4}%;width:{:.4}%;height:{:.4}%",
            self.x * 100.0,
            self.y * 100.0,
            self.width * 100.0,
            self.height * 100.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::TileRect;
    use crate::SplitAxis;

    #[test]
    fn a_split_divides_along_its_axis_without_gaps() {
        let (left, right) = TileRect::FULL.split(SplitAxis::Row, 0.25);
        assert_eq!(left, TileRect { x: 0.0, y: 0.0, width: 0.25, height: 1.0 });
        assert_eq!(right, TileRect { x: 0.25, y: 0.0, width: 0.75, height: 1.0 });

        let (top, bottom) = right.split(SplitAxis::Column, 0.5);
        assert!((top.height - 0.5).abs() < 1e-9);
        assert!((bottom.y - 0.5).abs() < 1e-9);
    }

    #[test]
    fn fractions_are_relative_to_the_rectangle() {
        let rect = TileRect { x: 0.5, y: 0.5, width: 0.5, height: 0.25 };
        let (fx, fy) = rect.fraction(0.75, 0.5625).expect("non-empty");
        assert!((fx - 0.5).abs() < 1e-9);
        assert!((fy - 0.25).abs() < 1e-9);
        assert!(rect.contains(0.75, 0.6));
        assert!(!rect.contains(0.25, 0.6));
    }

    #[test]
    fn the_longer_side_accounts_for_the_workspace_aspect() {
        let square = TileRect::FULL;
        assert_eq!(square.longer_axis(2.0), SplitAxis::Row);
        assert_eq!(square.longer_axis(0.5), SplitAxis::Column);
    }

    #[test]
    fn the_style_places_the_rectangle_in_percent() {
        let rect = TileRect { x: 0.5, y: 0.0, width: 0.5, height: 1.0 };
        assert_eq!(rect.style(), "left:50.0000%;top:0.0000%;width:50.0000%;height:100.0000%");
    }
}
