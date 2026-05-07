//! Pre-execution stateless checks on a script.
//!
//! These run once before the dispatch loop:
//! - Script size cap (10000 bytes).
//! - "Always-fail" opcodes that reject the script even when reached only in
//!   an unexecuted `OP_IF` branch. In legacy script (`SigVersion::Base`)
//!   this set is `OP_VERIF`, `OP_VERNOTIF`, and the disabled splice / bitwise
//!   / multiplicative / shift opcodes. (Tapscript narrows this set; that's
//!   M3's problem.)
//!
//! These checks are pure inspection of the script bytes — they do not
//! advance the interpreter or mutate any stack.

use crate::script::error::ScriptError;
use crate::script::opcodes::Opcode;
use crate::script::sig_version::SigVersion;

use super::reader::ScriptReader;

/// Maximum script length, in bytes (`MAX_SCRIPT_SIZE`).
pub const MAX_SCRIPT_SIZE: usize = 10_000;

/// Run pre-execution checks. Returns `Ok(())` if the script is admissible
/// for evaluation.
///
/// # Errors
/// - `ScriptSize` if the script exceeds 10000 bytes.
/// - `BadPushLength` if a push opcode is malformed.
/// - `BadOpcodeReserved` if `OP_VERIF` / `OP_VERNOTIF` appears anywhere.
/// - `DisabledOpcode` if a disabled opcode appears anywhere (legacy only).
pub fn pre_scan(script: &[u8], sig_version: SigVersion) -> Result<(), ScriptError> {
    if script.len() > MAX_SCRIPT_SIZE {
        return Err(ScriptError::ScriptSize);
    }

    let mut reader = ScriptReader::new(script);
    while let Some(step) = reader.next_step()? {
        match step.opcode {
            Opcode::VerIf | Opcode::VerNotIf => {
                return Err(ScriptError::BadOpcodeReserved);
            }
            op if op.is_disabled() && sig_version.is_base() => {
                return Err(ScriptError::DisabledOpcode);
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_script_passes() {
        assert!(pre_scan(&[], SigVersion::Base).is_ok());
    }

    #[test]
    fn size_cap() {
        let too_long = vec![0x00u8; MAX_SCRIPT_SIZE + 1];
        assert!(matches!(
            pre_scan(&too_long, SigVersion::Base),
            Err(ScriptError::ScriptSize)
        ));
        let ok = vec![0x00u8; MAX_SCRIPT_SIZE];
        assert!(pre_scan(&ok, SigVersion::Base).is_ok());
    }

    #[test]
    fn verif_rejected_anywhere() {
        // OP_IF OP_VERIF OP_ENDIF — VERIF is in an unexecuted branch but
        // still rejected.
        let script = [0x63, 0x65, 0x68];
        assert!(matches!(
            pre_scan(&script, SigVersion::Base),
            Err(ScriptError::BadOpcodeReserved)
        ));
    }

    #[test]
    fn vernotif_rejected_anywhere() {
        let script = [0x66];
        assert!(matches!(
            pre_scan(&script, SigVersion::Base),
            Err(ScriptError::BadOpcodeReserved)
        ));
    }

    #[test]
    fn disabled_opcode_rejected_in_legacy() {
        // OP_CAT (0x7e) — disabled in legacy.
        let script = [0x7e];
        assert!(matches!(
            pre_scan(&script, SigVersion::Base),
            Err(ScriptError::DisabledOpcode)
        ));
    }

    #[test]
    fn disabled_opcode_unexecuted_still_rejected_in_legacy() {
        // OP_IF OP_AND OP_ENDIF
        let script = [0x63, 0x84, 0x68];
        assert!(matches!(
            pre_scan(&script, SigVersion::Base),
            Err(ScriptError::DisabledOpcode)
        ));
    }

    #[test]
    fn disabled_opcode_allowed_in_tapscript_unexecuted() {
        // BIP342 narrows the disabled-set to "fail only if executed". The
        // pre-scan should not reject under SigVersion::Tapscript.
        let script = [0x84]; // OP_AND
        assert!(pre_scan(&script, SigVersion::Tapscript).is_ok());
    }

    #[test]
    fn malformed_push_propagates() {
        // OP_PUSHBYTES_5 with no data
        let script = [0x05];
        assert!(matches!(
            pre_scan(&script, SigVersion::Base),
            Err(ScriptError::BadPushLength)
        ));
    }

    #[test]
    fn long_well_formed_script_passes() {
        // 9000 bytes of OP_NOP — under cap, no banned opcodes.
        let script = vec![0x61u8; 9000];
        assert!(pre_scan(&script, SigVersion::Base).is_ok());
    }
}
