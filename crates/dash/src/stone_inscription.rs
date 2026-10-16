use crate::{CarveAlign, CarveFont, CarveText, Carving, ChainBlock, DisplayFormat, WallStone};

#[derive(Debug, Clone, PartialEq)]
pub struct StoneInscription {
    lines: Vec<CarveText>,
}

impl StoneInscription {
    const MARGIN: f64 = 0.16;
    const HEADING: f64 = 0.4;
    const LINE: f64 = 0.2;
    const LEADING: f64 = 1.25;
    const SHALLOW_ZEROS: f64 = 17.0;
    const DEEP_ZEROS: f64 = 24.0;
    const RICH_FEE_RATE: f64 = 8.0;

    #[must_use]
    pub fn new(stone: &WallStone, block: &ChainBlock) -> Self {
        let face = stone.face(Self::margin(stone));
        let depth = Self::depth(block);
        let centre = (face.left + face.right) * 0.5;
        let wanted = [
            (DisplayFormat::count(block.height as usize), Self::HEADING, block.fee_rate().is_some_and(|rate| rate >= Self::RICH_FEE_RATE)),
            (block.fee_rate().map_or_else(|| "NO FEES".to_owned(), |rate| format!("{rate:.1} SAT/VB")), Self::LINE, false),
        ];
        let height = face.height();
        let sized: Vec<(String, f64, bool)> = wanted
            .into_iter()
            .map(|(text, share, gilt)| {
                let fits = face.width() / (text.chars().count() as f64 * CarveFont::Display.advance()).max(1.0);
                let size = (height * share).min(fits);
                (text, size, gilt)
            })
            .collect();
        let mut kept = sized.len();
        while kept > 1 && sized[..kept].iter().map(|line| line.1 * Self::LEADING).sum::<f64>() > height {
            kept -= 1;
        }
        let block_height: f64 = sized[..kept].iter().map(|line| line.1 * Self::LEADING).sum();
        let mut baseline = face.top + (height - block_height) * 0.5;
        let lines = sized[..kept]
            .iter()
            .map(|(text, size, gilt)| {
                baseline += size * Self::LEADING;
                let carved = CarveText::new(text.clone(), CarveFont::Display, *size, centre, baseline - size * (Self::LEADING - 1.0) * 0.5 - size * 0.22).align(CarveAlign::Centre).depth(depth);
                if *gilt { carved.gilt() } else { carved }
            })
            .collect();
        Self { lines }
    }

    fn margin(stone: &WallStone) -> f64 {
        let face = stone.face(0.0);
        face.width().min(face.height()) * Self::MARGIN
    }

    fn depth(block: &ChainBlock) -> f64 {
        let zeros = block.hash.chars().take_while(|character| *character == '0').count() as f64;
        0.6 + 0.4 * ((zeros - Self::SHALLOW_ZEROS) / (Self::DEEP_ZEROS - Self::SHALLOW_ZEROS)).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn letters(&self) -> usize {
        self.lines.iter().map(|line| line.text.chars().count()).sum()
    }

    #[must_use]
    pub fn cut(&self, progress: f64) -> Vec<Carving> {
        let mut left = (self.letters() as f64 * progress.clamp(0.0, 1.0)).round() as usize;
        let mut carvings = Vec::new();
        for line in &self.lines {
            if left == 0 {
                break;
            }
            let count = line.text.chars().count();
            let shown = count.min(left);
            left -= shown;
            let full_width = line.width();
            let text: String = line.text.chars().take(shown).collect();
            let start = line.align.left_of(line.x, full_width);
            carvings.push(CarveText { text, x: start, align: CarveAlign::Start, ..line.clone() }.into());
        }
        carvings
    }

    #[must_use]
    pub fn lines(&self) -> &[CarveText] {
        &self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::StoneInscription;
    use crate::{CarveFill, Carving, ChainBlock, WallMason};

    fn block(height: u32, zeros: usize, fees: u64) -> ChainBlock {
        let hash = format!("{}{}", "0".repeat(zeros), "a".repeat(64 - zeros));
        serde_json::from_str(&format!(
            r#"{{"height":{height},"hash":"{hash}","time":1,"difficulty":null,"transactions":3001,"weight":3996000,"subsidy":312500000,"fees":{fees}}}"#
        ))
        .expect("block")
    }

    fn stone(span: f64) -> crate::WallStone {
        WallMason::new(1000.0, 120.0, 3).lay(0, &[span]).remove(0)
    }

    #[test]
    fn every_line_sits_inside_the_stone_face() {
        for span in [90.0, 200.0, 420.0] {
            let stone = stone(span);
            let face = stone.face(0.0);
            let inscription = StoneInscription::new(&stone, &block(903_412, 19, 20_000_000));

            assert!(!inscription.lines().is_empty());
            assert!(inscription.lines().iter().all(|line| face.contains(line.bounds())), "span {span}");
        }
    }

    #[test]
    fn a_roomy_stone_carries_the_height_over_the_fee_rate() {
        let inscription = StoneInscription::new(&stone(420.0), &block(903_412, 19, 20_000_000));
        let texts: Vec<&str> = inscription.lines().iter().map(|line| line.text.as_str()).collect();

        assert_eq!(texts, ["903,412", "20.0 SAT/VB"]);
        assert!(inscription.lines()[0].size > inscription.lines()[1].size);
        assert_eq!(inscription.lines()[1].fill, CarveFill::Plain);
    }

    #[test]
    fn stones_of_the_same_course_carve_their_height_at_the_same_size() {
        let sizes: Vec<f64> = [300.0, 360.0, 420.0].iter().map(|span| StoneInscription::new(&stone(*span), &block(903_412, 19, 1)).lines()[0].size).collect();

        assert!(sizes.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 1e-9), "{sizes:?}");
    }

    #[test]
    fn a_cramped_stone_drops_lines_from_the_bottom_but_keeps_the_height() {
        let small = StoneInscription::new(&stone(84.0), &block(903_412, 19, 20_000_000));

        assert_eq!(small.lines()[0].text, "903,412");
    }

    #[test]
    fn a_high_fee_block_has_a_gilded_height_and_a_cheap_one_does_not() {
        let rich = StoneInscription::new(&stone(420.0), &block(1, 19, 20_000_000));
        let cheap = StoneInscription::new(&stone(420.0), &block(1, 19, 1_000_000));

        assert_eq!(rich.lines()[0].fill, CarveFill::Gilt);
        assert_eq!(cheap.lines()[0].fill, CarveFill::Plain);
    }

    #[test]
    fn more_leading_zeros_cut_deeper() {
        let shallow = StoneInscription::new(&stone(420.0), &block(1, 17, 1));
        let deep = StoneInscription::new(&stone(420.0), &block(1, 24, 1));

        assert!(deep.lines()[0].depth > shallow.lines()[0].depth);
        assert!((deep.lines()[0].depth - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_cut_reveals_letters_in_reading_order_without_moving_them() {
        let inscription = StoneInscription::new(&stone(420.0), &block(903_412, 19, 20_000_000));
        let whole = inscription.cut(1.0);
        let start = inscription.cut(3.0 / inscription.letters() as f64);

        assert!(inscription.cut(0.0).is_empty());
        assert_eq!(whole.len(), inscription.lines().len());
        let (Carving::Text(first), Carving::Text(complete)) = (&start[0], &whole[0]) else { panic!("text") };
        assert_eq!(first.text, "903");
        assert!((first.x - complete.x).abs() < 1e-9);
    }
}
