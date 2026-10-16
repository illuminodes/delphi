use crate::{Carving, LimestonePatch, StoneFace, WallLight, WallStone};

#[derive(Debug, Clone, PartialEq)]
pub struct StoneSketch {
    pub key: String,
    pub stone: WallStone,
    pub offset: (f64, f64),
    pub alpha: f64,
    pub shade: f64,
    pub patch: LimestonePatch,
    pub tone: f64,
    pub tint: [f64; 3],
    pub light: WallLight,
    pub carvings: Vec<Carving>,
    pub letters: usize,
}

impl StoneSketch {
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.key.bytes().fold(0xCBF2_9CE4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3))
    }

    #[must_use]
    pub fn face(&self, scale: f64) -> StoneFace {
        StoneFace::new(&self.stone, scale, self.tone, self.tint, self.light, self.seed())
    }

    #[must_use]
    pub fn cache_key(&self, scale: f64) -> String {
        let bounds = self.stone.bounds();
        format!("{}:{}:{:.0}x{:.0}@{:.0},{:.0}:{scale}", self.key, self.letters, bounds.width(), bounds.height(), bounds.left, bounds.top)
    }
}

#[cfg(test)]
mod tests {
    use super::StoneSketch;
    use crate::{Limestone, WallLight, WallMason};

    fn sketch(key: &str, letters: usize) -> StoneSketch {
        StoneSketch {
            key: key.to_owned(),
            stone: WallMason::new(800.0, 100.0, 1).lay(0, &[200.0]).remove(0),
            offset: (0.0, 0.0),
            alpha: 1.0,
            shade: 0.0,
            patch: Limestone::patch(1),
            tone: 1.0,
            tint: [1.0; 3],
            light: WallLight::new((0.0, 0.0), 1000.0),
            carvings: Vec::new(),
            letters,
        }
    }

    #[test]
    fn the_cache_key_changes_with_the_letters_cut_and_the_seed_with_the_block() {
        assert_ne!(sketch("a", 3).cache_key(2.0), sketch("a", 4).cache_key(2.0));
        assert_eq!(sketch("a", 3).cache_key(2.0), sketch("a", 3).cache_key(2.0));
        assert_ne!(sketch("a", 3).seed(), sketch("b", 3).seed());
    }
}
