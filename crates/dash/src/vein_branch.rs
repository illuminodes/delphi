use crate::{BranchMask, BranchNode};

#[derive(Debug, Clone, PartialEq)]
pub struct VeinBranch {
    nodes: Vec<BranchNode>,
}

impl VeinBranch {
    #[must_use]
    pub const fn new(nodes: Vec<BranchNode>) -> Self {
        Self { nodes }
    }

    #[must_use]
    pub fn nodes(&self) -> &[BranchNode] {
        &self.nodes
    }

    pub fn stamp(&self, mask: &mut BranchMask) {
        for pair in self.nodes.windows(2) {
            mask.stamp(pair[0], pair[1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VeinBranch;
    use crate::{BranchMask, BranchNode};

    #[test]
    fn a_branch_stamps_every_one_of_its_segments() {
        let node = |x, y| BranchNode { x, y, half_width: 1.5 };
        let branch = VeinBranch::new(vec![node(5.0, 5.0), node(30.0, 5.0), node(30.0, 35.0)]);
        let mut mask = BranchMask::new(40, 40, 1.0);
        branch.stamp(&mut mask);

        assert!(mask.at(17, 4) > 0.5 || mask.at(17, 5) > 0.5);
        assert!(mask.at(29, 20) > 0.5 || mask.at(30, 20) > 0.5);
        assert!(mask.at(15, 25) < 1e-6);
    }
}
