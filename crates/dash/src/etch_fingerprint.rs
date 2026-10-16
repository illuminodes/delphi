#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EtchFingerprint {
    hash: u64,
}

impl EtchFingerprint {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;

    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self { hash: bytes.iter().fold(Self::OFFSET, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(Self::PRIME)) }
    }

    #[must_use]
    pub const fn hash(&self) -> u64 {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::EtchFingerprint;
    use crate::{CarveCut, CarveFill, CarveGroove, CarveMask, MarbleKind, StoneEtch, WireframePlan};

    struct EtchedSample;

    impl EtchedSample {
        const WIDTH: usize = 160;
        const HEIGHT: usize = 96;
        const SCALE: f64 = 2.0;

        fn mask(kind: MarbleKind) -> CarveMask {
            let mut mask = CarveMask::new(Self::WIDTH, Self::HEIGHT, Self::SCALE);
            for row in 20..60 {
                for column in 20..34 {
                    mask.press(column, row, 1.0, 1.0, CarveFill::Plain, CarveCut::Relief);
                }
                for column in 60..74 {
                    mask.press(column, row, 1.0, 1.0, CarveFill::Gilt, CarveCut::Relief);
                }
                for column in 100..103 {
                    mask.press(column, row, 0.8, 1.0, CarveFill::Plain, CarveCut::Incised);
                }
            }
            mask.thicken_relief(kind.etch().thicken);
            mask.stamp_groove(&CarveGroove::line((5.0, 38.0), (75.0, 38.0), 2.0));
            mask.stamp_groove(&CarveGroove::ring((60.0, 20.0), 9.0, 1.5));
            mask
        }

        fn stone() -> Vec<u8> {
            (0..Self::WIDTH * Self::HEIGHT)
                .flat_map(|index| {
                    let (column, row) = (index % Self::WIDTH, index / Self::WIDTH);
                    let vein = ((column * 7 + row * 13) % 97) as u8;
                    [40 + vein, 38 + vein / 2, 45 + (row % 31) as u8, 255]
                })
                .collect()
        }

        fn etched(kind: MarbleKind) -> EtchFingerprint {
            let mut pixels = Self::stone();
            StoneEtch::new(kind.etch(), 7).apply(&mut pixels, &Self::mask(kind));
            EtchFingerprint::of(&pixels)
        }

        fn plan(kind: MarbleKind) -> EtchFingerprint {
            EtchFingerprint::of(format!("{:?}", WireframePlan::new(1440.0, 900.0, kind, 7).carvings()).as_bytes())
        }
    }

    #[test]
    fn the_same_bytes_give_the_same_fingerprint_and_one_changed_byte_does_not() {
        assert_eq!(EtchFingerprint::of(b"marble"), EtchFingerprint::of(b"marble"));
        assert_ne!(EtchFingerprint::of(b"marble"), EtchFingerprint::of(b"marblf"));
    }

    #[test]
    fn the_etched_shading_is_unchanged() {
        for (kind, expected) in [(MarbleKind::Nero, 16_420_835_365_971_611_829), (MarbleKind::Parian, 7_034_565_260_980_661_389)] {
            assert_eq!(EtchedSample::etched(kind).hash(), expected, "{kind:?} etched pixels changed");
        }
    }

    #[test]
    fn the_etched_layout_is_unchanged() {
        for (kind, expected) in [(MarbleKind::Nero, 11_951_218_725_577_435_567), (MarbleKind::Parian, 5_149_479_015_482_557_485)] {
            assert_eq!(EtchedSample::plan(kind).hash(), expected, "{kind:?} carving plan changed");
        }
    }
}
