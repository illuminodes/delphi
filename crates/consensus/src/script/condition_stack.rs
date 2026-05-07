//! `ConditionStack` — O(1) "are we currently executing?" tracker.
//!
//! Bitcoin Core's interpreter scans a `vector<bool>` on every opcode to decide
//! whether the current `OP_IF`/`OP_NOTIF` branch is being executed. That scan
//! is O(N) in nesting depth and was a known DoS-able surface (bitslog 2017,
//! Core PR review club #16902). We track the same information in O(1) by
//! caching:
//!
//! - `depth_total`     — how many `IF`/`NOTIF` are currently open.
//! - `depth_executing` — how many of those have `true` (executing) on top.
//!
//! "Are we executing?" is `depth_total == depth_executing`. Push, pop, and
//! `OP_ELSE`'s toggle each update both counters in O(1).
//!
//! For the toggle we also track the bool that was last pushed at the current
//! top, in a small inline buffer. The interpreter only ever toggles the
//! topmost entry (`OP_ELSE`), so a single `Vec<bool>` with O(1) ops is
//! sufficient — but the `executing()` query never reads from it.

/// Tracks `OP_IF`/`OP_NOTIF` nesting with an O(1) "are we executing?" query.
#[derive(Debug, Clone)]
pub struct ConditionStack {
    /// One bool per open `IF`/`NOTIF`, in nesting order. Used only by `toggle_top`
    /// (i.e. `OP_ELSE`) and `pop` (i.e. `OP_ENDIF`); never scanned for the
    /// hot-path `executing()` query.
    branches: Vec<bool>,
    /// Number of `true` entries in `branches`. Maintained incrementally.
    depth_executing: u32,
}

/// Pre-allocated capacity for `branches`. Most legacy scripts have
/// shallow IF nesting (≤ 4-8 levels); 16 covers typical use without
/// burning RAM, and avoids Vec reallocs in the hot path. Deeper scripts
/// grow it.
const INITIAL_BRANCHES_CAPACITY: usize = 16;

impl Default for ConditionStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ConditionStack {
    /// Empty — no `OP_IF` is currently open. Pre-allocates a small
    /// capacity so typical-depth IF nesting doesn't realloc.
    #[must_use]
    pub fn new() -> Self {
        Self {
            branches: Vec::with_capacity(INITIAL_BRANCHES_CAPACITY),
            depth_executing: 0,
        }
    }

    /// Total open `IF`/`NOTIF` depth.
    #[inline]
    #[must_use]
    pub fn depth(&self) -> u32 {
        // `branches.len()` is bounded by script size (≤ 10000 bytes ⇒ ≤ 10000
        // opcodes ⇒ ≤ 10000 nested ifs). Fits a u32.
        #[allow(clippy::cast_possible_truncation)]
        { self.branches.len() as u32 }
    }

    /// `true` if every open branch is taking the executing side.
    /// O(1).
    #[inline]
    #[must_use]
    pub fn executing(&self) -> bool {
        self.depth() == self.depth_executing
    }

    /// `true` if no `IF`/`NOTIF` is currently open.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    /// Push the result of an `OP_IF`/`OP_NOTIF` condition.
    pub fn push(&mut self, executing: bool) {
        self.branches.push(executing);
        if executing {
            self.depth_executing += 1;
        }
    }

    /// Pop the topmost branch (`OP_ENDIF`). No-op if the stack is empty —
    /// callers detect that via `is_empty` and return `UnbalancedConditional`.
    pub fn pop(&mut self) -> Option<bool> {
        let top = self.branches.pop()?;
        if top {
            self.depth_executing -= 1;
        }
        Some(top)
    }

    /// Flip the topmost branch (`OP_ELSE`). `None` if the stack is empty.
    pub fn toggle_top(&mut self) -> Option<()> {
        let top = self.branches.last_mut()?;
        if *top {
            self.depth_executing -= 1;
        } else {
            self.depth_executing += 1;
        }
        *top = !*top;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_executing() {
        let cs = ConditionStack::new();
        assert!(cs.executing());
        assert!(cs.is_empty());
        assert_eq!(cs.depth(), 0);
    }

    #[test]
    fn push_true_keeps_executing() {
        let mut cs = ConditionStack::new();
        cs.push(true);
        assert!(cs.executing());
        assert_eq!(cs.depth(), 1);
    }

    #[test]
    fn push_false_stops_executing() {
        let mut cs = ConditionStack::new();
        cs.push(false);
        assert!(!cs.executing());
    }

    #[test]
    fn nested_true_then_false_not_executing() {
        let mut cs = ConditionStack::new();
        cs.push(true);
        cs.push(false);
        assert!(!cs.executing());
        assert_eq!(cs.depth(), 2);
    }

    #[test]
    fn pop_restores_outer() {
        let mut cs = ConditionStack::new();
        cs.push(true);
        cs.push(false);
        assert!(!cs.executing());
        cs.pop();
        assert!(cs.executing());
    }

    #[test]
    fn toggle_flips_top() {
        let mut cs = ConditionStack::new();
        cs.push(false);
        assert!(!cs.executing());
        cs.toggle_top();
        assert!(cs.executing());
        cs.toggle_top();
        assert!(!cs.executing());
    }

    #[test]
    fn toggle_inner_under_outer_false_stays_not_executing() {
        // Outer false dominates — toggling the inner can't enable execution.
        let mut cs = ConditionStack::new();
        cs.push(false);
        cs.push(true);
        assert!(!cs.executing());
        cs.toggle_top();
        assert!(!cs.executing());
    }

    #[test]
    fn toggle_inner_under_outer_true_can_execute() {
        let mut cs = ConditionStack::new();
        cs.push(true);
        cs.push(false);
        assert!(!cs.executing());
        cs.toggle_top();
        assert!(cs.executing());
    }

    #[test]
    fn deeply_nested_o1() {
        // Push 10000 true entries — `executing()` must still be O(1) and true.
        let mut cs = ConditionStack::new();
        for _ in 0..10_000 {
            cs.push(true);
        }
        assert!(cs.executing());
        assert_eq!(cs.depth(), 10_000);

        // One false at depth 10000 makes us non-executing in O(1).
        cs.push(false);
        assert!(!cs.executing());
    }

    #[test]
    fn pop_empty_returns_none() {
        let mut cs = ConditionStack::new();
        assert!(cs.pop().is_none());
    }

    #[test]
    fn toggle_empty_returns_none() {
        let mut cs = ConditionStack::new();
        assert!(cs.toggle_top().is_none());
    }

    #[test]
    fn depth_executing_invariant_after_mix() {
        // After arbitrary push/pop/toggle, depth_executing must equal the
        // count of `true` entries.
        let mut cs = ConditionStack::new();
        cs.push(true);
        cs.push(false);
        cs.push(true);
        cs.toggle_top(); // top is now false
        cs.push(false);
        cs.toggle_top(); // top is now true
        // branches: [true, false, false, true]; executing count = 2.
        assert_eq!(cs.depth_executing, 2);
        assert!(!cs.executing()); // depth=4, executing=2
    }
}
