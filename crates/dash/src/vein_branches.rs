use crate::{BranchGrowth, BranchMask, StoneRecipe, VeinBranch};

#[derive(Debug, Clone, PartialEq)]
pub struct VeinBranches {
    branches: Vec<VeinBranch>,
}

impl VeinBranches {
    const SALT: u64 = 0x5851_F42D_4C95_7F2D;

    #[must_use]
    pub fn grow(course: &[(f64, f64)], recipe: StoneRecipe, seed: u64) -> Self {
        let mut growth = BranchGrowth::new(seed ^ Self::SALT);
        for _ in 0..recipe.branch_count {
            growth.sprout_from(course, recipe.branch_width);
        }
        Self { branches: growth.finish() }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.branches.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    #[must_use]
    pub fn mask(&self, width: usize, height: usize, scale: f64) -> BranchMask {
        let mut mask = BranchMask::new(width, height, scale);
        for branch in &self.branches {
            branch.stamp(&mut mask);
        }
        mask
    }
}

#[cfg(test)]
mod tests {
    use super::VeinBranches;
    use crate::MarbleKind;

    struct BranchesFixture;

    impl BranchesFixture {
        fn course() -> Vec<(f64, f64)> {
            (0..=60).map(|step| (f64::from(step) * 10.0, 150.0 + 40.0 * (f64::from(step) * 0.3).sin())).collect()
        }
    }

    #[test]
    fn every_stone_grows_at_least_as_many_branches_as_its_recipe_asks() {
        for kind in MarbleKind::ALL {
            let recipe = kind.recipe();
            assert!(VeinBranches::grow(&BranchesFixture::course(), recipe, 7).len() >= recipe.branch_count, "{}", kind.key());
        }
    }

    #[test]
    fn the_mask_shows_branches_but_leaves_most_of_the_stone_bare() {
        let branches = VeinBranches::grow(&BranchesFixture::course(), MarbleKind::Nero.recipe(), 7);
        let mask = branches.mask(600, 300, 1.0);
        let covered = (0..300).flat_map(|row| (0..600).map(move |column| (column, row))).filter(|(column, row)| mask.at(*column, *row) > 0.3).count();

        assert!(covered > 500 && covered < 600 * 300 / 8, "{covered}");
    }
}
