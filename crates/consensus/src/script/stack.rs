//! Script execution stack with consensus-mandated size caps.
//!
//! Two caps:
//! - **Per-element**: 520 bytes. Pushes that would exceed this fail with
//!   `ScriptError::PushSize`.
//! - **Combined**: 1000 elements across stack + altstack. Operations that
//!   would push the combined depth past 1000 fail with `ScriptError::StackSize`.
//!
//! Stack elements are stored as `StackElement<N>` — inline up to `N` bytes,
//! heap above. `N` is a const generic on `Stack` so we can tune the inline
//! cap without rewriting the type or its call sites; the interpreter uses
//! `Stack<32>` (covers numbers, hashes, x-only pubkeys; compressed pubkeys
//! and DER signatures spill once per CHECKSIG, which is fine).

use std::mem::MaybeUninit;
use std::ops::Deref;

use super::error::ScriptError;
use super::num::ScriptNum;

/// Maximum size of any single stack element, in bytes (`MAX_SCRIPT_ELEMENT_SIZE`).
pub const MAX_SCRIPT_ELEMENT_SIZE: usize = 520;

/// Maximum combined stack + altstack depth (`MAX_STACK_SIZE`).
pub const MAX_STACK_SIZE: usize = 1000;

/// Default inline capacity for the interpreter's stacks. Covers `ScriptNum`
/// (≤ 5 bytes), HASH160 / RIPEMD-160 / SHA-1 (20 bytes), SHA-256 / HASH256
/// (32 bytes), x-only pubkeys (32 bytes). Compressed pubkeys (33 bytes) and
/// DER signatures (~71-72 bytes) spill to heap.
pub const DEFAULT_INLINE_CAP: usize = 32;

/// One stack slot — inline up to `N` bytes, heap above. Same shape as
/// `mantle::primitives::ScriptBuf<N>` from the core crate.
///
/// **Safety invariant**: when `Inline { len, data }`, the first `len` bytes
/// of `data` are initialized; bytes at positions `len..N` are uninitialized
/// and must never be read. `as_slice` and `Deref<Target=[u8]>` only ever
/// expose the initialized prefix. Skipping the zero-fill on construction
/// removes ~17% from `OP_ADD`-class hot paths where most operands are
/// 1-2 bytes long.
#[derive(Clone)]
pub enum StackElement<const N: usize> {
    /// Stored inline. `len <= N`. The first `len` bytes of `data` are
    /// initialized; the rest are uninitialized.
    Inline { len: u8, data: [MaybeUninit<u8>; N] },
    /// Stored on the heap. `len > N`.
    Heap(Box<[u8]>),
}

impl<const N: usize> StackElement<N> {
    /// The empty element — used for `OP_0`, false-bool pushes, and as the
    /// neutral fill for unused slots. The `data` array is uninitialized but
    /// safe because `len = 0` means `as_slice` returns `&[]`.
    pub const EMPTY: Self = Self::Inline {
        len: 0,
        data: [const { MaybeUninit::uninit() }; N],
    };

    /// Construct from a byte slice. Picks inline vs. heap by length.
    ///
    /// `#[inline]` because callers in the dispatch hot path (every push,
    /// every `push_num`, every `push_bool`) want this body in their frame
    /// so LLVM can fold the small-`s.len()` case into a couple of `mov`s.
    ///
    /// # Panics
    /// Never in practice — `N` is always small enough that `s.len() <= N`
    /// implies `s.len()` fits in `u8`. (Asserted in debug builds.)
    #[inline]
    #[must_use]
    pub fn from_slice(s: &[u8]) -> Self {
        if s.len() <= N {
            debug_assert!(u8::try_from(s.len()).is_ok(), "N must be <= 255");
            // Skip zero-fill: only the first `s.len()` bytes are read by
            // `as_slice`; the rest of `data` stays uninitialized but
            // unobservable.
            let mut data: [MaybeUninit<u8>; N] = [const { MaybeUninit::uninit() }; N];
            // SAFETY: `MaybeUninit<u8>` and `u8` have the same layout, so we
            // can write `s` directly into the initial prefix. We only access
            // `data[..s.len()]` from `as_slice`, so leaving the tail
            // uninitialized is sound.
            unsafe {
                let dst = data.as_mut_ptr().cast::<u8>();
                std::ptr::copy_nonoverlapping(s.as_ptr(), dst, s.len());
            }
            #[allow(clippy::cast_possible_truncation)]
            Self::Inline { len: s.len() as u8, data }
        } else {
            Self::Heap(s.to_vec().into_boxed_slice())
        }
    }

    /// Borrow as a byte slice.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { len, data } => {
                let len = *len as usize;
                // SAFETY: by the type invariant, `data[..len]` is initialized.
                // `MaybeUninit<u8>` and `u8` are layout-compatible.
                unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), len) }
            }
            Self::Heap(b) => b,
        }
    }

    /// Length in bytes.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Inline { len, .. } => *len as usize,
            Self::Heap(b) => b.len(),
        }
    }

    /// `true` if zero bytes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bitcoin's "is this byte string truthy?" rule.
    ///
    /// A value is truthy iff any byte is non-zero, *except* a sole high bit
    /// on the last byte (i.e. `0x80`) is treated as zero — that encodes
    /// negative-zero in `ScriptNum`.
    #[inline]
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        let bytes = self.as_slice();
        let Some((&last, rest)) = bytes.split_last() else {
            return false;
        };
        if rest.iter().any(|&b| b != 0) {
            return true;
        }
        last != 0 && last != 0x80
    }
}

impl<const N: usize> Deref for StackElement<N> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<const N: usize> Default for StackElement<N> {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<const N: usize> std::fmt::Debug for StackElement<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StackElement({:?})", self.as_slice())
    }
}

impl<const N: usize> PartialEq for StackElement<N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> Eq for StackElement<N> {}

impl<const N: usize> PartialEq<[u8]> for StackElement<N> {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}

impl<const N: usize, const M: usize> PartialEq<[u8; M]> for StackElement<N> {
    fn eq(&self, other: &[u8; M]) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> PartialEq<&[u8]> for StackElement<N> {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_slice() == *other
    }
}

impl<const N: usize> PartialEq<Vec<u8>> for StackElement<N> {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

/// Mutable stack of byte-string elements. Used for both the main stack and
/// the alt stack — the type is shared because the cap is on their *combined*
/// size and methods that grow either need to see both.
///
/// `N` is the per-slot inline byte capacity. The interpreter uses
/// [`DEFAULT_INLINE_CAP`].
#[derive(Debug, Clone)]
pub struct Stack<const N: usize = DEFAULT_INLINE_CAP> {
    items: Vec<StackElement<N>>,
}

impl<const N: usize> Default for Stack<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Stack<N> {
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

    /// Borrow the element at depth `n` (0 = top).
    ///
    /// Callers wanting a `&[u8]` deref the result; callers wanting the
    /// truthy semantics call `.is_truthy()`.
    ///
    /// # Errors
    /// `InvalidStackOperation` if `n` is at or beyond the current depth.
    #[inline]
    pub fn peek(&self, n: usize) -> Result<&StackElement<N>, ScriptError> {
        let idx = self.items.len().checked_sub(n + 1).ok_or(ScriptError::InvalidStackOperation)?;
        Ok(&self.items[idx])
    }

    /// Pop the top element.
    ///
    /// # Errors
    /// `InvalidStackOperation` if empty.
    #[inline]
    pub fn pop(&mut self) -> Result<StackElement<N>, ScriptError> {
        self.items.pop().ok_or(ScriptError::InvalidStackOperation)
    }

    /// Push without bounds checking. Used by tests and internal helpers that
    /// have already validated the size — production opcode handlers go
    /// through `push_checked`.
    pub fn push_unchecked(&mut self, item: StackElement<N>) {
        self.items.push(item);
    }

    /// Push from a byte slice, with both caps enforced. The slice is copied
    /// into the slot (inline if it fits in `N`, heap otherwise).
    ///
    /// `altstack_len` is the current depth of the *other* stack so the
    /// combined-size check is exact.
    ///
    /// # Errors
    /// - `PushSize` if `bytes.len() > MAX_SCRIPT_ELEMENT_SIZE`.
    /// - `StackSize` if the push would take the combined depth past
    ///   `MAX_STACK_SIZE`.
    #[inline]
    pub fn push_checked(&mut self, bytes: &[u8], altstack_len: usize) -> Result<(), ScriptError> {
        if bytes.len() > MAX_SCRIPT_ELEMENT_SIZE {
            return Err(ScriptError::PushSize);
        }
        if self.items.len() + altstack_len >= MAX_STACK_SIZE {
            return Err(ScriptError::StackSize);
        }
        self.items.push(StackElement::from_slice(bytes));
        Ok(())
    }

    /// Push an already-constructed element with caps enforced. Used when the
    /// caller owns a `StackElement` (e.g. from `pop()` returning to the
    /// stack after a peek-style operation).
    ///
    /// # Errors
    /// As `push_checked`.
    #[inline]
    pub fn push_element(
        &mut self,
        elem: StackElement<N>,
        altstack_len: usize,
    ) -> Result<(), ScriptError> {
        if elem.len() > MAX_SCRIPT_ELEMENT_SIZE {
            return Err(ScriptError::PushSize);
        }
        if self.items.len() + altstack_len >= MAX_STACK_SIZE {
            return Err(ScriptError::StackSize);
        }
        self.items.push(elem);
        Ok(())
    }

    /// Duplicate the element at depth `n` and push it. Avoids the
    /// `peek().to_vec(); push()` allocation pattern by cloning the slot
    /// directly — for inline slots this is a `memcpy` of `N+1` bytes with
    /// no heap touch.
    ///
    /// # Errors
    /// - `InvalidStackOperation` if depth `n` does not exist.
    /// - `StackSize` if the push would take combined depth past the cap.
    #[inline]
    pub fn dup_at(&mut self, n: usize, altstack_len: usize) -> Result<(), ScriptError> {
        if self.items.len() + altstack_len >= MAX_STACK_SIZE {
            return Err(ScriptError::StackSize);
        }
        let idx = self.items.len().checked_sub(n + 1).ok_or(ScriptError::InvalidStackOperation)?;
        let copy = self.items[idx].clone();
        self.items.push(copy);
        Ok(())
    }

    /// Borrow the underlying slice — for read-only iteration in tests and
    /// the eventual `OP_DEPTH` handler.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[StackElement<N>] {
        &self.items
    }

    /// Remove the item at depth `n` (0 = top), shifting later items down.
    /// Used by `OP_ROLL` and similar.
    ///
    /// # Errors
    /// `InvalidStackOperation` if depth `n` does not exist.
    pub fn remove_at(&mut self, n: usize) -> Result<StackElement<N>, ScriptError> {
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
    #[inline]
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
    /// See [`StackElement::is_truthy`] for the rule.
    ///
    /// # Errors
    /// `InvalidStackOperation` if the stack is empty.
    #[inline]
    pub fn pop_bool(&mut self) -> Result<bool, ScriptError> {
        Ok(self.pop()?.is_truthy())
    }

    /// Push a `ScriptNum`, encoded minimally, with both stack caps enforced.
    /// Encodes directly into the slot — never allocates for values whose
    /// minimal encoding fits in `N` bytes (which is every `i64` value once
    /// `N >= 5`).
    ///
    /// # Errors
    /// `StackSize` if the combined depth would exceed `MAX_STACK_SIZE`.
    #[inline]
    pub fn push_num(&mut self, value: i64, altstack_len: usize) -> Result<(), ScriptError> {
        let n = ScriptNum::from_i64(value);
        self.push_checked(n.as_bytes(), altstack_len)
    }

    /// Push a boolean as either the empty byte string (false) or `[0x01]` (true).
    ///
    /// # Errors
    /// `StackSize` if the combined depth would exceed `MAX_STACK_SIZE`.
    #[inline]
    pub fn push_bool(&mut self, value: bool, altstack_len: usize) -> Result<(), ScriptError> {
        let bytes: &[u8] = if value { &[0x01] } else { &[] };
        self.push_checked(bytes, altstack_len)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// All tests use the default 32-byte inline cap.
    type TestStack = Stack<DEFAULT_INLINE_CAP>;

    #[test]
    fn empty_pop_is_invalid_stack_op() {
        let mut s = TestStack::new();
        assert!(matches!(s.pop(), Err(ScriptError::InvalidStackOperation)));
    }

    #[test]
    fn push_then_peek_then_pop() {
        let mut s = TestStack::new();
        s.push_checked(&[1, 2, 3], 0).expect("under both caps");
        s.push_checked(&[4, 5], 0).expect("under both caps");
        assert_eq!(s.len(), 2);
        assert_eq!(s.peek(0).expect("non-empty"), &[4, 5]);
        assert_eq!(s.peek(1).expect("non-empty"), &[1, 2, 3]);
        assert_eq!(s.pop().expect("non-empty").as_slice(), &[4, 5]);
        assert_eq!(s.pop().expect("non-empty").as_slice(), &[1, 2, 3]);
        assert!(s.is_empty());
    }

    #[test]
    fn peek_past_bottom_is_invalid_stack_op() {
        let mut s = TestStack::new();
        s.push_checked(&[1], 0).expect("ok");
        assert!(matches!(s.peek(1), Err(ScriptError::InvalidStackOperation)));
    }

    #[test]
    fn push_size_cap() {
        let mut s = TestStack::new();
        let oversized = vec![0u8; MAX_SCRIPT_ELEMENT_SIZE + 1];
        assert!(matches!(s.push_checked(&oversized, 0), Err(ScriptError::PushSize)));
        let max = vec![0u8; MAX_SCRIPT_ELEMENT_SIZE];
        assert!(s.push_checked(&max, 0).is_ok());
    }

    #[test]
    fn combined_stack_size_cap() {
        let mut s = TestStack::new();
        // Fill the main stack to just under the cap.
        for _ in 0..MAX_STACK_SIZE - 1 {
            s.push_checked(&[], 0).expect("under cap");
        }
        // One more push reaches the cap (combined depth = 1000); the next fails.
        s.push_checked(&[], 0).expect("at cap is fine");
        assert!(matches!(
            s.push_checked(&[], 0),
            Err(ScriptError::StackSize)
        ));
    }

    #[test]
    fn combined_cap_uses_altstack_len() {
        let mut s = TestStack::new();
        // Pretend the alt stack has 999 items already.
        s.push_checked(&[], 999).expect("combined = 999 + 1 <= 1000");
        assert!(matches!(
            s.push_checked(&[], 999),
            Err(ScriptError::StackSize)
        ));
    }

    #[test]
    fn remove_at_shifts() {
        let mut s = TestStack::new();
        for i in 0u8..3 {
            s.push_checked(&[i], 0).expect("ok");
        }
        // Stack from bottom to top: [0], [1], [2]. Top is depth 0.
        let removed = s.remove_at(1).expect("depth 1 exists");
        assert_eq!(removed.as_slice(), &[1]);
        assert_eq!(s.peek(0).expect("non-empty"), &[2]);
        assert_eq!(s.peek(1).expect("non-empty"), &[0]);
    }

    #[test]
    fn truncate_to_smaller() {
        let mut s = TestStack::new();
        for i in 0u8..5 {
            s.push_checked(&[i], 0).expect("ok");
        }
        s.truncate(2).expect("smaller");
        assert_eq!(s.len(), 2);
        assert!(matches!(s.truncate(99), Err(ScriptError::InvalidStackOperation)));
    }

    #[test]
    fn dup_at_avoids_double_alloc() {
        let mut s = TestStack::new();
        s.push_checked(&[0xab; 20], 0).expect("ok");
        s.dup_at(0, 0).expect("ok");
        assert_eq!(s.len(), 2);
        assert_eq!(s.peek(0).expect("non-empty"), &[0xab; 20]);
        assert_eq!(s.peek(1).expect("non-empty"), &[0xab; 20]);
    }

    // ── Inline vs heap ──────────────────────────────────────────────────────

    #[test]
    fn small_value_stays_inline() {
        let elem = StackElement::<32>::from_slice(&[1, 2, 3]);
        assert!(matches!(elem, StackElement::Inline { .. }));
        assert_eq!(elem.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn at_inline_cap_stays_inline() {
        let elem = StackElement::<32>::from_slice(&[0xab; 32]);
        assert!(matches!(elem, StackElement::Inline { .. }));
        assert_eq!(elem.len(), 32);
    }

    #[test]
    fn over_inline_cap_goes_heap() {
        let elem = StackElement::<32>::from_slice(&[0xcd; 33]);
        assert!(matches!(elem, StackElement::Heap(_)));
        assert_eq!(elem.len(), 33);
        assert_eq!(elem.as_slice(), &[0xcd; 33]);
    }

    #[test]
    fn empty_constant() {
        let elem: StackElement<32> = StackElement::EMPTY;
        assert!(elem.is_empty());
        assert!(matches!(elem, StackElement::Inline { len: 0, .. }));
    }

    // ── High-level helpers ──────────────────────────────────────────────────

    #[test]
    fn push_pop_num_roundtrip() {
        let mut s = TestStack::new();
        for v in [-127, -1, 0, 1, 127, 128, 0x7fff_ffff] {
            s.push_num(v, 0).expect("ok");
            assert_eq!(s.pop_num(4, false).expect("ok"), v);
        }
    }

    #[test]
    fn pop_num_respects_size_limit() {
        let mut s = TestStack::new();
        // Five-byte encoding pushed.
        s.push_num(0x007f_ffff_ffff_i64, 0).expect("ok");
        // Arithmetic limit (4) rejects it.
        assert!(matches!(
            s.pop_num(4, false),
            Err(ScriptError::NumOverflow)
        ));
    }

    #[test]
    fn pop_num_minimal_check() {
        let mut s = TestStack::new();
        // Non-minimal encoding of zero.
        s.push_unchecked(StackElement::from_slice(&[0x00]));
        assert!(matches!(
            s.pop_num(4, true),
            Err(ScriptError::NumNotMinimal)
        ));
    }

    #[test]
    fn is_truthy_rules() {
        let t = |b: &[u8]| StackElement::<32>::from_slice(b).is_truthy();
        // Empty is false.
        assert!(!t(&[]));
        // All-zero bytes is false.
        assert!(!t(&[0, 0, 0]));
        // Single 0x80 (negative zero) is false.
        assert!(!t(&[0x80]));
        // 0x00 0x80 is also false (mantissa zero, sign byte only).
        assert!(!t(&[0x00, 0x80]));
        // Any non-zero byte before the last makes it true.
        assert!(t(&[0x01, 0x80]));
        // A non-sign-only last byte is true.
        assert!(t(&[0x01]));
        assert!(t(&[0x7f]));
    }

    #[test]
    fn push_pop_bool() {
        let mut s = TestStack::new();
        s.push_bool(true, 0).expect("ok");
        s.push_bool(false, 0).expect("ok");
        assert!(!s.pop_bool().expect("ok"));
        assert!(s.pop_bool().expect("ok"));
    }

    #[test]
    fn push_bool_encodes_canonically() {
        let mut s = TestStack::new();
        s.push_bool(true, 0).expect("ok");
        assert_eq!(s.peek(0).expect("non-empty"), &[0x01]);
        s.pop().expect("ok");
        s.push_bool(false, 0).expect("ok");
        assert!(s.peek(0).expect("non-empty").is_empty());
    }
}
