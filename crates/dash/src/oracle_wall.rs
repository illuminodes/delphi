use crate::{ChainBlock, OracleLayout, SplitMix, WallMason, WallStone};

#[derive(Debug, Clone, PartialEq)]
pub struct OracleWall {
    blocks: Vec<ChainBlock>,
    kept_course: usize,
    ground_course: usize,
    seed: u64,
}

impl OracleWall {
    const VARIETY: (f64, f64) = (0.7, 1.3);

    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { blocks: Vec::new(), kept_course: 0, ground_course: 0, seed }
    }

    pub fn push(&mut self, block: ChainBlock, layout: &OracleLayout) -> bool {
        if self.blocks.last().is_some_and(|last| last.hash == block.hash) {
            return false;
        }
        self.blocks.push(block);
        let top = self.lay(layout).iter().map(|stone| stone.course).max().unwrap_or(self.ground_course);
        let before = self.ground_course;
        self.ground_course = self.ground_course.max((top + 1).saturating_sub(OracleLayout::COURSES));
        self.ground_course > before
    }

    pub fn forget_hidden(&mut self, layout: &OracleLayout) {
        let hidden = self.lay(layout).iter().filter(|stone| stone.course < self.ground_course).count();
        self.blocks.drain(..hidden);
        self.kept_course = self.ground_course;
    }

    #[must_use]
    pub const fn ground_course(&self) -> usize {
        self.ground_course
    }

    #[must_use]
    pub fn has_hidden(&self) -> bool {
        self.kept_course < self.ground_course
    }

    #[must_use]
    pub fn blocks(&self) -> &[ChainBlock] {
        &self.blocks
    }

    #[must_use]
    pub fn newest(&self) -> Option<&ChainBlock> {
        self.blocks.last()
    }

    #[must_use]
    pub fn stones(&self, layout: &OracleLayout, view: f64) -> Vec<WallStone> {
        let wall = layout.wall();
        let lift = view * layout.course_height();
        self.lay(layout).into_iter().map(|stone| stone.shifted(wall.left, wall.bottom + lift)).collect()
    }

    #[must_use]
    pub fn joins_parent(&self, at: usize) -> bool {
        at > 0 && self.blocks.get(at).zip(self.blocks.get(at - 1)).is_some_and(|(block, parent)| block.previous == parent.hash)
    }

    fn lay(&self, layout: &OracleLayout) -> Vec<WallStone> {
        let spans: Vec<f64> = self.blocks.iter().map(|block| layout.span(block.fullness()) * Self::variety(&block.hash)).collect();
        WallMason::new(layout.wall().width(), layout.course_height(), self.seed).lay(self.kept_course, &spans)
    }

    fn variety(hash: &str) -> f64 {
        let seed = hash.bytes().fold(0xCBF2_9CE4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3));
        Self::VARIETY.0 + (Self::VARIETY.1 - Self::VARIETY.0) * SplitMix::new(seed).unit()
    }
}

#[cfg(test)]
mod tests {
    use super::OracleWall;
    use crate::{ChainBlock, OracleLayout};

    fn block(height: u32, weight: u64) -> ChainBlock {
        serde_json::from_str(&format!(
            r#"{{"height":{height},"hash":"h{height}","previous":"h{}","time":1,"difficulty":null,"transactions":2000,"weight":{weight}}}"#,
            height - 1
        ))
        .expect("block")
    }

    fn layout() -> OracleLayout {
        OracleLayout::new(1200.0, 800.0)
    }

    fn built(count: u32) -> OracleWall {
        let mut wall = OracleWall::new(5);
        for height in 1..=count {
            wall.push(block(height, 3_990_000), &layout());
            wall.forget_hidden(&layout());
        }
        wall
    }

    #[test]
    fn every_stone_lies_inside_the_wall_area_once_the_view_has_followed() {
        let mut wall = OracleWall::new(5);
        for height in 1..40 {
            wall.push(block(height, 3_990_000 - u64::from(height % 5) * 600_000), &layout());
            wall.forget_hidden(&layout());
            let area = layout().wall().grow(layout().course_height() * 0.15);
            let view = wall.ground_course() as f64;
            assert!(wall.stones(&layout(), view).iter().all(|stone| area.contains(stone.bounds())), "after {height}");
        }
    }

    #[test]
    fn the_wall_never_shows_more_than_three_courses_and_keeps_the_newest_blocks() {
        let wall = built(30);

        assert!(wall.ground_course() > 0);
        assert_eq!(wall.newest().map(|block| block.height), Some(30));
        assert!(wall.blocks().len() >= 7 && wall.blocks().len() <= 15, "{}", wall.blocks().len());
        assert!(wall.blocks().windows(2).all(|pair| pair[1].height == pair[0].height + 1));
    }

    #[test]
    fn a_course_pushed_out_of_view_is_kept_until_forgotten_and_nothing_moves_when_it_goes() {
        let mut wall = OracleWall::new(5);
        let mut height = 0;
        let rose = loop {
            height += 1;
            if wall.push(block(height, 3_990_000), &layout()) {
                break true;
            }
        };
        let view = wall.ground_course() as f64;
        let before = wall.stones(&layout(), view);
        let count = wall.blocks().len();

        assert!(rose && wall.has_hidden());
        wall.forget_hidden(&layout());
        let after = wall.stones(&layout(), view);

        assert!(!wall.has_hidden() && wall.blocks().len() < count);
        assert_eq!(&before[before.len() - after.len()..].iter().map(|stone| (&stone.top, &stone.bottom)).collect::<Vec<_>>(), &after.iter().map(|stone| (&stone.top, &stone.bottom)).collect::<Vec<_>>());
    }

    #[test]
    fn the_newest_stone_sits_highest_and_the_oldest_lowest() {
        let wall = built(20);
        let stones = wall.stones(&layout(), wall.ground_course() as f64);
        let (first, last) = (stones.first().expect("stone"), stones.last().expect("stone"));

        assert!(last.centre().1 < first.centre().1);
        assert!((first.bottom[0].1 - layout().wall().bottom).abs() <= layout().course_height() * 0.12 + 1e-9);
    }

    #[test]
    fn stones_of_full_blocks_still_differ_in_width() {
        let wall = built(8);
        let widths: Vec<f64> = wall.stones(&layout(), 0.0).iter().filter(|stone| stone.course == 0).map(|stone| stone.bounds().width()).collect();
        let (low, high) = widths[..widths.len() - 1].iter().fold((f64::INFINITY, 0.0_f64), |(low, high), width| (low.min(*width), high.max(*width)));

        assert!(high - low > 30.0, "{widths:?}");
    }

    #[test]
    fn a_repeated_block_is_ignored_and_a_child_joins_its_parent() {
        let mut wall = OracleWall::new(5);
        wall.push(block(7, 3_000_000), &layout());
        wall.push(block(7, 3_000_000), &layout());
        wall.push(block(8, 3_000_000), &layout());

        assert_eq!(wall.blocks().len(), 2);
        assert!(wall.joins_parent(1) && !wall.joins_parent(0));
    }
}
