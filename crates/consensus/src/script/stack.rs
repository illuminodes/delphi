//! Script execution stack with consensus-mandated size caps.
//!
//! Two caps:
//! - **Per-element**: 520 bytes. Pushes that would exceed this fail with
//!   `ScriptError::PushSize`.
//! - **Combined**: 1000 elements across stack + altstack. Operations that
//!   would push the combined depth past 1000 fail with `ScriptError::StackSize`.
//!
//! Both caps are enforced by the methods that grow either stack. The
//! interpreter calls `Stack::push_checked` rather than poking the inner
//! `Vec` directly.

use super::error::ScriptError;

/// Maximum size of any single stack element, in bytes (`MAX_SCRIPT_ELEMENT_SIZE`).
pub const MAX_SCRIPT_ELEMENT_SIZE: usize = 520;

/// Maximum combined stack + altstack depth (`MAX_STACK_SIZE`).
pub const MAX_STACK_SIZE: usize = 1000;

/// Mutable stack of byte-string elements. Used for both the main stack and
/// the alt stack — the type is shared because the cap is on their *combined*
/// size and methods that grow either need to see both.
#[derive(Debug, Clone, Default)]
pub struct Stack {
    items: Vec<Vec<u8>>,
}

impl Stack {
    /// Empty stack.
    #[must_use]
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Number of items.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `true` if no items.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Borrow the top element. `n = 0` is the topmost.
    ///
    /// # Errors
    /// `InvalidStackOperation` if `n` is at or beyond the current depth.
    pub fn peek(&self, n: usize) -> Result<&[u8], ScriptError> {
        let idx = self.items.len().checked_sub(n + 1).ok_or(ScriptError::InvalidStackOperation)?;
        Ok(&self.items[idx])
    }

    /// Borrow the top element mutably. `n = 0` is the topmost.
    ///
    /// # Errors
    /// `InvalidStackOperation` if the stack does not have `n+1` items.
    pub fn peek_mut(&mut self, n: usize) -> Result<&mut Vec<u8>, ScriptError> {
        let idx = self.items.len().checked_sub(n + 1).ok_or(ScriptError::InvalidStackOperation)?;
        Ok(&mut self.items[idx])
    }

    /// Pop the top element.
    ///
    /// # Errors
    /// `InvalidStackOperation` if empty.
    pub fn pop(&mut self) -> Result<Vec<u8>, ScriptError> {
        self.items.pop().ok_or(ScriptError::InvalidStackOperation)
    }

    /// Push without bounds checking. Used by tests and internal helpers that
    /// have already validated the size — production opcode handlers go
    /// through `push_checked`.
    pub fn push_unchecked(&mut self, item: Vec<u8>) {
        self.items.push(item);
    }

    /// Push with both caps enforced. `altstack_len` is the current depth of
    /// the *other* stack so the combined-size check is exact.
    ///
    /// # Errors
    /// - `PushSize` if `item.len() > MAX_SCRIPT_ELEMENT_SIZE`.
    /// - `StackSize` if the push would take the combined depth past
    ///   `MAX_STACK_SIZE`.
    pub fn push_checked(&mut self, item: Vec<u8>, altstack_len: usize) -> Result<(), ScriptError> {
        if item.len() > MAX_SCRIPT_ELEMENT_SIZE {
            return Err(ScriptError::PushSize);
        }
        if self.items.len() + altstack_len >= MAX_STACK_SIZE {
            return Err(ScriptError::StackSize);
        }
        self.items.push(item);
        Ok(())
    }

    /// Borrow the underlying slice — for read-only iteration in tests and
    /// the eventual `OP_DEPTH` handler.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[Vec<u8>] {
        &self.items
    }

    /// Remove the item at depth `n` (0 = top), shifting later items down.
    /// Used by `OP_ROLL` and similar.
    ///
    /// # Errors
    /// `InvalidStackOperation` if depth `n` does not exist.
    pub fn remove_at(&mut self, n: usize) -> Result<Vec<u8>, ScriptError> {
        let idx = self.items.len().checked_sub(n + 1).ok_or(ScriptError::InvalidStackOperation)?;
        Ok(self.items.remove(idx))
    }

    /// Truncate to `new_len` items.
    ///
    /// # Errors
    /// `InvalidStackOperation` if `new_len > self.len()`.
    pub fn truncate(&mut self, new_len: usize) -> Result<(), ScriptError> {
        if new_len > self.items.len() {
            return Err(ScriptError::InvalidStackOperation);
        }
        self.items.truncate(new_len);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pop_is_invalid_stack_op() {
        let mut s = Stack::new();
        assert!(matches!(s.pop(), Err(ScriptError::InvalidStackOperation)));
    }

    #[test]
    fn push_then_peek_then_pop() {
        let mut s = Stack::new();
        s.push_checked(vec![1, 2, 3], 0).expect("under both caps");
        s.push_checked(vec![4, 5], 0).expect("under both caps");
        assert_eq!(s.len(), 2);
        assert_eq!(s.peek(0).expect("non-empty"), &[4, 5]);
        assert_eq!(s.peek(1).expect("non-empty"), &[1, 2, 3]);
        assert_eq!(s.pop().expect("non-empty"), vec![4, 5]);
        assert_eq!(s.pop().expect("non-empty"), vec![1, 2, 3]);
        assert!(s.is_empty());
    }

    #[test]
    fn peek_past_bottom_is_invalid_stack_op() {
        let mut s = Stack::new();
        s.push_checked(vec![1], 0).expect("ok");
        assert!(matches!(s.peek(1), Err(ScriptError::InvalidStackOperation)));
    }

    #[test]
    fn push_size_cap() {
        let mut s = Stack::new();
        let oversized = vec![0u8; MAX_SCRIPT_ELEMENT_SIZE + 1];
        assert!(matches!(s.push_checked(oversized, 0), Err(ScriptError::PushSize)));
        let max = vec![0u8; MAX_SCRIPT_ELEMENT_SIZE];
        assert!(s.push_checked(max, 0).is_ok());
    }

    #[test]
    fn combined_stack_size_cap() {
        let mut s = Stack::new();
        // Fill the main stack to just under the cap.
        for _ in 0..MAX_STACK_SIZE - 1 {
            s.push_checked(vec![], 0).expect("under cap");
        }
        // One more push reaches the cap (combined depth = 1000); the next fails.
        s.push_checked(vec![], 0).expect("at cap is fine");
        assert!(matches!(
            s.push_checked(vec![], 0),
            Err(ScriptError::StackSize)
        ));
    }

    #[test]
    fn combined_cap_uses_altstack_len() {
        let mut s = Stack::new();
        // Pretend the alt stack has 999 items already.
        s.push_checked(vec![], 999).expect("combined = 999 + 1 <= 1000");
        assert!(matches!(
            s.push_checked(vec![], 999),
            Err(ScriptError::StackSize)
        ));
    }

    #[test]
    fn remove_at_shifts() {
        let mut s = Stack::new();
        for i in 0u8..3 {
            s.push_checked(vec![i], 0).expect("ok");
        }
        // Stack from bottom to top: [0], [1], [2]. Top is depth 0.
        let removed = s.remove_at(1).expect("depth 1 exists");
        assert_eq!(removed, vec![1]);
        assert_eq!(s.peek(0).expect("non-empty"), &[2]);
        assert_eq!(s.peek(1).expect("non-empty"), &[0]);
    }

    #[test]
    fn truncate_to_smaller() {
        let mut s = Stack::new();
        for i in 0u8..5 {
            s.push_checked(vec![i], 0).expect("ok");
        }
        s.truncate(2).expect("smaller");
        assert_eq!(s.len(), 2);
        assert!(matches!(s.truncate(99), Err(ScriptError::InvalidStackOperation)));
    }
}
