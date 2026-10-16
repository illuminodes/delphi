use crate::StoneRgb;

#[derive(Debug, Clone, PartialEq)]
pub struct CutFloor {
    width: usize,
    height: usize,
    stone: Vec<u8>,
    reach: f64,
}

impl CutFloor {
    #[must_use]
    pub fn new(width: usize, height: usize, stone: &[u8], reach: f64) -> Self {
        Self { width, height, stone: stone.to_vec(), reach }
    }

    #[must_use]
    pub fn at(&self, column: usize, row: usize, sink: f64) -> StoneRgb {
        let shift = (self.reach * sink).round() as usize;
        self.texel((column + shift).min(self.width - 1), (row + shift).min(self.height - 1))
    }

    fn texel(&self, column: usize, row: usize) -> StoneRgb {
        let index = (row * self.width + column) * 4;
        self.stone
            .get(index..index + 3)
            .map_or(StoneRgb::new([0.0; 3]), |pixel| StoneRgb::new([f64::from(pixel[0]), f64::from(pixel[1]), f64::from(pixel[2])]))
    }
}

#[cfg(test)]
mod tests {
    use super::CutFloor;

    struct FloorFixture;

    impl FloorFixture {
        const SIZE: usize = 20;

        fn ramp() -> Vec<u8> {
            (0..Self::SIZE * Self::SIZE).flat_map(|index| [(index % Self::SIZE * 10) as u8, (index / Self::SIZE * 10) as u8, 0, 255]).collect()
        }
    }

    #[test]
    fn the_rim_of_a_cut_shows_the_stone_right_under_it() {
        let floor = CutFloor::new(FloorFixture::SIZE, FloorFixture::SIZE, &FloorFixture::ramp(), 4.0);

        assert_eq!(floor.at(5, 7, 0.0).rgba(), [50, 70, 0, 255]);
    }

    #[test]
    fn the_bottom_of_a_cut_shows_the_stone_shifted_away_from_the_light() {
        let floor = CutFloor::new(FloorFixture::SIZE, FloorFixture::SIZE, &FloorFixture::ramp(), 4.0);

        assert_eq!(floor.at(5, 7, 1.0).rgba(), [90, 110, 0, 255]);
    }

    #[test]
    fn a_part_way_sink_lands_on_a_whole_grain_so_fine_speckle_is_not_blurred() {
        let floor = CutFloor::new(FloorFixture::SIZE, FloorFixture::SIZE, &FloorFixture::ramp(), 4.0);

        assert_eq!(floor.at(5, 7, 0.4).rgba(), [70, 90, 0, 255]);
    }

    #[test]
    fn a_shift_past_the_edge_holds_the_last_grain() {
        let floor = CutFloor::new(FloorFixture::SIZE, FloorFixture::SIZE, &FloorFixture::ramp(), 8.0);

        assert_eq!(floor.at(18, 18, 1.0).rgba(), [190, 190, 0, 255]);
    }
}
