/// Where a dragged card lands on another: beside one of its edges, or in its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropZone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

impl DropZone {
    const EDGE: f64 = 0.25;

    /// The zone under a pointer at fractions `x`, `y` of the card's width and height.
    #[must_use]
    pub fn at(x: f64, y: f64) -> Self {
        let edges = [(x, Self::Left), (1.0 - x, Self::Right), (y, Self::Top), (1.0 - y, Self::Bottom)];
        let (distance, zone) = edges
            .into_iter()
            .fold((f64::INFINITY, Self::Center), |best, edge| if edge.0 < best.0 { edge } else { best });
        if distance > Self::EDGE {
            Self::Center
        } else {
            zone
        }
    }

    /// Whether the dropped card goes before the card it lands on.
    #[must_use]
    pub const fn leads(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }

    #[must_use]
    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Center => "center",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DropZone;

    #[test]
    fn the_nearest_edge_wins_and_the_middle_swaps() {
        assert_eq!(DropZone::at(0.05, 0.5), DropZone::Left);
        assert_eq!(DropZone::at(0.95, 0.4), DropZone::Right);
        assert_eq!(DropZone::at(0.5, 0.1), DropZone::Top);
        assert_eq!(DropZone::at(0.4, 0.9), DropZone::Bottom);
        assert_eq!(DropZone::at(0.5, 0.5), DropZone::Center);
    }

    #[test]
    fn leading_edges_go_before_the_target() {
        assert!(DropZone::Top.leads());
        assert!(DropZone::Left.leads());
        assert!(!DropZone::Right.leads());
    }
}
