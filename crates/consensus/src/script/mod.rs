//! Bitcoin script interpreter.
//!
//! See `docs/architecture/script.md` for the staged plan. This module is being
//! built milestone-by-milestone; M1.0 ships only the foundation types
//! (`ScriptError`, `ScriptFlags`, `Opcode`, `ScriptNum`, `Stack`). The
//! interpreter loop and `verify_script` entry point land in M1.1+.

pub mod checker;
pub mod condition_stack;
pub mod error;
pub mod flags;
pub mod interpreter;
pub mod num;
pub mod opcodes;
pub mod sig_version;
pub mod stack;

pub use checker::{NopSigChecker, SignatureChecker};
pub use condition_stack::ConditionStack;
pub use error::ScriptError;
pub use flags::ScriptFlags;
pub use interpreter::Interpreter;
pub use num::ScriptNum;
pub use opcodes::Opcode;
pub use sig_version::SigVersion;
pub use stack::Stack;
