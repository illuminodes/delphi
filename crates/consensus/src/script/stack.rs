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
use super::num::ScriptNum;

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
    /// Empty stack pre-sized to the consensus cap so opcode handlers don't
    /// pay reallocation cost on growth.
    #[must_use]
    pub fn new() -> Self {
        Self { items: Vec::with_capacity(MAX_STACK_SIZE) }
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

    // ── High-level helpers used by opcode handlers ──────────────────────────

    /// Pop the top element and decode it as a `ScriptNum` operand.
    ///
    /// `max_len` is the per-opcode size limit (4 for arithmetic, 5 for
    /// `OP_CHECKLOCKTIMEVERIFY`); `require_minimal` is the `MINIMALDATA`
    /// gate.
    ///
    /// # Errors
    /// - `InvalidStackOperation` if the stack is empty.
    /// - `NumOverflow` / `NumNotMinimal` per `ScriptNum::from_bytes`.
    pub fn pop_num(
        &mut self,
        max_len: usize,
        require_minimal: bool,
    ) -> Result<i64, ScriptError> {
        let bytes = self.pop()?;
        Ok(ScriptNum::from_bytes(&bytes, max_len, require_minimal)?.to_i64())
    }

    /// Pop the top element and interpret it as a Bitcoin-script boolean.
    ///
    /// A value is "truthy" iff any byte is non-zero, *except* the last byte
    /// having only the sign bit set (i.e. `0x80`) is treated as zero — that
    /// encodes negative-zero in `ScriptNum`.
    ///
    /// # Errors
    /// `InvalidStackOperation` if the stack is empty.
    pub fn pop_bool(&mut self) -> Result<bool, ScriptError> {
        let bytes = self.pop()?;
        Ok(Self::cast_to_bool(&bytes))
    }

    /// Push a `ScriptNum`, encoded minimally, with both stack caps enforced.
    ///
    /// # Errors
    /// `StackSize` if the combined depth would exceed `MAX_STACK_SIZE`.
    pub fn push_num(&mut self, value: i64, altstack_len: usize) -> Result<(), ScriptError> {
        let n = ScriptNum::from_i64(value);
        self.push_checked(n.as_bytes().to_vec(), altstack_len)
    }

    /// Push a boolean as either the empty byte string (false) or `[0x01]` (true).
    ///
    /// # Errors
    /// `StackSize` if the combined depth would exceed `MAX_STACK_SIZE`.
    pub fn push_bool(&mut self, value: bool, altstack_len: usize) -> Result<(), ScriptError> {
        let bytes = if value { vec![0x01] } else { Vec::new() };
        self.push_checked(bytes, altstack_len)
    }

    /// Bitcoin's "is this byte string truthy?" rule. Public so callers that
    /// already hold the bytes (e.g. `OP_IF` peeking at the top of stack) can
    /// reuse the rule without round-tripping through pop/push.
    #[must_use]
    pub fn cast_to_bool(bytes: &[u8]) -> bool {
        let Some((&last, rest)) = bytes.split_last() else {
            return false;
        };
        if rest.iter().any(|&b| b != 0) {
            return true;
        }
        // Only the most-significant byte is non-zero. It's truthy unless it's
        // the sign-only byte 0x80 (which encodes negative zero).
        last != 0 && last != 0x80
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

    // ── High-level helpers ──────────────────────────────────────────────────

    #[test]
    fn push_pop_num_roundtrip() {
        let mut s = Stack::new();
        for v in [-127, -1, 0, 1, 127, 128, 0x7fff_ffff] {
            s.push_num(v, 0).expect("ok");
            assert_eq!(s.pop_num(4, false).expect("ok"), v);
        }
    }

    #[test]
    fn pop_num_respects_size_limit() {
        let mut s = Stack::new();
        // Five-byte encoding pushed.
        s.push_num(0x7fff_ffff_ffi64, 0).expect("ok");
        // Arithmetic limit (4) rejects it.
        assert!(matches!(
            s.pop_num(4, false),
            Err(ScriptError::NumOverflow)
        ));
    }

    #[test]
    fn pop_num_minimal_check() {
        let mut s = Stack::new();
        // Non-minimal encoding of zero.
        s.push_unchecked(vec![0x00]);
        assert!(matches!(
            s.pop_num(4, true),
            Err(ScriptError::NumNotMinimal)
        ));
    }

    #[test]
    fn cast_to_bool_rules() {
        // Empty is false.
        assert!(!Stack::cast_to_bool(&[]));
        // All-zero bytes is false.
        assert!(!Stack::cast_to_bool(&[0, 0, 0]));
        // Single 0x80 (negative zero) is false.
        assert!(!Stack::cast_to_bool(&[0x80]));
        // 0x00 0x80 is also false (mantissa zero, sign byte only).
        assert!(!Stack::cast_to_bool(&[0x00, 0x80]));
        // Any non-zero byte before the last makes it true.
        assert!(Stack::cast_to_bool(&[0x01, 0x80]));
        // A non-sign-only last byte is true.
        assert!(Stack::cast_to_bool(&[0x01]));
        assert!(Stack::cast_to_bool(&[0x7f]));
    }

    #[test]
    fn push_pop_bool() {
        let mut s = Stack::new();
        s.push_bool(true, 0).expect("ok");
        s.push_bool(false, 0).expect("ok");
        assert_eq!(s.pop_bool().expect("ok"), false);
        assert_eq!(s.pop_bool().expect("ok"), true);
    }

    #[test]
    fn push_bool_encodes_canonically() {
        let mut s = Stack::new();
        s.push_bool(true, 0).expect("ok");
        assert_eq!(s.peek(0).expect("non-empty"), &[0x01]);
        s.pop().expect("ok");
        s.push_bool(false, 0).expect("ok");
        assert!(s.peek(0).expect("non-empty").is_empty());
    }
}
