use mantle::primitives::{BlockHeight, Hash256, OutPoint, Satoshi};

use crate::chain_view::{ChainView, ConnectResult, ConnectScratch, NewCoin};
use crate::error::ValidationError;
use crate::script::sighash::Bip143Cache;
use crate::script::{ScriptExecutionCache, ScriptFlags, ScriptVerifier, verify_script_cached};
use crate::tx_view::{BlockView, InputView, OutputView, TxView};

// ── Constants ─────────────────────────────────────────────────────────────────

const INITIAL_SUBSIDY: u64 = 50 * 100_000_000;
const HALVING_INTERVAL: u32 = 210_000;

/// Maximum legacy sigops per block.
const MAX_BLOCK_SIGOPS: u32 = 20_000;

/// Heights of the two historical BIP30 exception blocks on mainnet.
/// These coinbase txids collide with earlier unspent coinbases but were
/// accepted by Core before BIP30 enforcement began.
const BIP30_EXCEPTION_HEIGHTS: [u32; 2] = [91_842, 91_880];

// ── connect_block ─────────────────────────────────────────────────────────────

/// Stage 3: connect a block to the chain, checking UTXO-dependent rules.
///
/// Assumes Stage 1 and Stage 2 have already passed. Checks:
/// - BIP30: no tx shares a txid with an existing unspent coinbase
/// - Every non-coinbase input exists in the UTXO set
/// - Coinbase maturity (100 blocks)
/// - Fee math: sum(inputs) >= sum(outputs) per tx; coinbase <= subsidy + fees
/// - Legacy sigops budget (<= 20,000 per block)
///
/// Returns a [`ConnectResult`] carrying the UTXO delta for the storage layer.
///
/// `scratch` is reused across txid computations to avoid repeated allocation.
///
/// `skip_script` — when `true`, script verification is skipped entirely for
/// all inputs. Set this for blocks at or below the `assumevalid` tip: all
/// other consensus checks (UTXO, maturity, fee math, sigops) still run.
///
/// `exec_cache` — optional [`ScriptExecutionCache`]. When `Some`, each input
/// probes the cache before running the interpreter; hits short-circuit the
/// full verification. Pass `None` to disable caching (fuzzer, tests).
///
/// `precomputed_txids` — optional pre-hashed per-tx ids in block order. The
/// pipeline computes txids before validation (for intra-block dep collection,
/// BIP30 probes, etc.); passing them here lets `connect_block` skip its own
/// txid pass and reuse the same hashes. Length must equal `block.tx_count()`.
/// Pass `None` to compute internally (legacy / single-block test paths).
///
/// # Errors
/// Returns the first [`ValidationError`] encountered.
///
/// # Panics
/// Panics if `precomputed_txids` is `Some(slice)` and `slice.len() != block.tx_count()`.
#[allow(clippy::too_many_lines)] // single-pass per-input loop reads better as one function
#[allow(clippy::too_many_arguments)] // precomputed_txids is opt-in; refactoring to a struct adds churn
pub fn connect_block<'blk, B, V>(
    block: &'blk B,
    height: BlockHeight,
    view: &V,
    scratch: &mut Vec<u8>,
    connect_scratch: &mut ConnectScratch,
    skip_script: bool,
    mut exec_cache: Option<&mut ScriptExecutionCache>,
    precomputed_txids: Option<&[Hash256]>,
) -> Result<ConnectResult<'blk>, ValidationError>
where
    B: BlockView,
    V: ChainView,
{
    if let Some(txids) = precomputed_txids {
        assert_eq!(
            txids.len(),
            block.tx_count(),
            "precomputed txids length must match block tx count",
        );
    }
    let is_bip30_exception = BIP30_EXCEPTION_HEIGHTS.contains(&height.0);
    let subsidy = block_subsidy(height);
    let script_flags = ScriptFlags::for_height(height);

    // Pre-size created. Most blocks have O(thousands) of outputs.
    let total_outputs: usize = block.txs().map(TxView::output_count).sum();
    let mut created: Vec<NewCoin> = Vec::with_capacity(total_outputs);

    // Reuse caller-owned scratch vecs; capacity accumulates across blocks.
    connect_scratch.spent.clear();
    let outpoints_scratch = &mut connect_scratch.outpoints;

    let mut coins_scratch: Vec<Option<&crate::chain_view::Coin>> = Vec::new();

    let mut total_fees = Satoshi::ZERO;
    let mut total_sigops: u32 = 0;

    for (tx_index, tx) in block.txs().enumerate() {
        let is_coinbase = tx_index == 0;
        let txid = match precomputed_txids {
            Some(txids) => txids[tx_index],
            None => tx.compute_txid(scratch)?,
        };

        // BIP30: if any output of this tx is at a txid that already has an
        // unspent coinbase in the UTXO set, reject (unless this is an exception height).
        if !is_bip30_exception {
            let bip30_probe = OutPoint { txid, vout: 0 };
            if let Some(coin) = view.get_coin(&bip30_probe)?
                && coin.coinbase
            {
                return Err(ValidationError::Bip30Violation { tx_index });
            }
        }

        // Legacy sigops: count OP_CHECKSIG / OP_CHECKMULTISIG in scriptSig and
        // scriptPubKey. Each OP_CHECKMULTISIG counts as 20 (MAX_PUBKEYS_PER_MULTISIG).
        for input in tx.inputs() {
            total_sigops = total_sigops.saturating_add(count_legacy_sigops(input.script_sig()));
        }
        for output in tx.outputs() {
            total_sigops = total_sigops.saturating_add(count_legacy_sigops(output.script_pubkey()));
        }
        if total_sigops > MAX_BLOCK_SIGOPS {
            return Err(ValidationError::SigopsLimitExceeded {
                count: total_sigops,
            });
        }

        if is_coinbase {
            // Coinbase outputs go straight into created; no input lookup needed.
            for (vout, output) in tx.outputs().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                created.push(NewCoin {
                    outpoint: OutPoint {
                        txid,
                        vout: vout as u32,
                    },
                    value: output.value(),
                    script_pubkey: output.script_pubkey(),
                    coinbase: true,
                    height,
                });
            }
            // Coinbase reward is validated after we know total_fees (below).
            continue;
        }

        // Non-coinbase: look up every input in the UTXO set. Reuse both
        // scratch vecs instead of allocating per tx.
        outpoints_scratch.clear();
        outpoints_scratch.extend(tx.inputs().map(|i| i.previous_output()));
        view.get_coins_into(outpoints_scratch, &mut coins_scratch)?;
        let coins = &coins_scratch;

        // Built once per-tx only when scripts run without an exec_cache.
        // Under skip_script or when exec_cache is present, never allocated.
        let bip143_cache: Option<Bip143Cache> = if !skip_script && exec_cache.is_none() {
            Some(Bip143Cache::new(tx))
        } else {
            None
        };

        let mut input_total = Satoshi::ZERO;
        for (input_index, (input, coin_opt)) in tx.inputs().zip(coins.iter()).enumerate() {
            let coin = coin_opt.as_ref().ok_or(ValidationError::MissingInput {
                tx_index,
                input_index,
            })?;

            // Coinbase maturity: coinbase outputs must be 100 blocks deep.
            if coin.coinbase {
                let depth = height.0.saturating_sub(coin.height.0);
                if depth < 100 {
                    return Err(ValidationError::CoinbaseMaturityViolation {
                        tx_index,
                        input_index,
                    });
                }
            }

            // Script verification: fourth per-input check, after coin lookup,
            // before fee math. Skipped entirely under assumevalid (`skip_script`).
            // When a cache is provided, probes it before running the interpreter.
            if !skip_script {
                let script_result = if let Some(ref mut cache) = exec_cache {
                    verify_script_cached(
                        input.script_sig(),
                        coin.script_pubkey.as_slice(),
                        tx,
                        input_index,
                        coin.value,
                        script_flags,
                        cache,
                    )
                } else {
                    ScriptVerifier::with_cache(
                        tx,
                        input_index,
                        coin.value,
                        script_flags,
                        #[allow(clippy::clone_on_ref_ptr)]
                        bip143_cache.clone().unwrap_or_else(|| unreachable!()),
                    )
                    .verify(input.script_sig(), coin.script_pubkey.as_slice())
                };
                script_result.map_err(|kind| ValidationError::ScriptVerifyFailed {
                    tx_index,
                    input_index,
                    kind,
                })?;
            }

            input_total = input_total
                .checked_add(coin.value)
                .ok_or(ValidationError::InputValueOverflow { tx_index })?;

            connect_scratch.spent.push(input.previous_output());
        }

        let output_total = tx
            .outputs()
            .try_fold(Satoshi::ZERO, |acc, o| acc.checked_add(o.value()))
            .ok_or(ValidationError::InputValueOverflow { tx_index })?;

        if output_total.to_u64() > input_total.to_u64() {
            return Err(ValidationError::NegativeFee { tx_index });
        }

        let fee = Satoshi::new(input_total.to_u64() - output_total.to_u64());
        total_fees = total_fees
            .checked_add(fee)
            .ok_or(ValidationError::InputValueOverflow { tx_index })?;

        for (vout, output) in tx.outputs().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            created.push(NewCoin {
                outpoint: OutPoint {
                    txid,
                    vout: vout as u32,
                },
                value: output.value(),
                script_pubkey: output.script_pubkey(),
                coinbase: false,
                height,
            });
        }
    }

    // Validate coinbase reward against subsidy + fees.
    let coinbase_output_total = block
        .tx(0)
        .outputs()
        .try_fold(Satoshi::ZERO, |acc, o| acc.checked_add(o.value()))
        .ok_or(ValidationError::CoinbaseRewardExceedsSubsidy)?;

    let max_coinbase = subsidy
        .checked_add(total_fees)
        .ok_or(ValidationError::CoinbaseRewardExceedsSubsidy)?;

    if coinbase_output_total.to_u64() > max_coinbase.to_u64() {
        return Err(ValidationError::CoinbaseRewardExceedsSubsidy);
    }

    let spent = connect_scratch.spent.drain(..).collect();
    Ok(ConnectResult {
        created,
        spent,
        fees: total_fees,
    })
}

// ── BlockHeightExt ────────────────────────────────────────────────────────────

/// Extension trait that adds `subsidy()` to [`BlockHeight`].
pub trait BlockHeightExt {
    /// Block reward subsidy in satoshis at this height.
    fn subsidy(self) -> Satoshi;
}

impl BlockHeightExt for BlockHeight {
    fn subsidy(self) -> Satoshi {
        block_subsidy(self)
    }
}

// ── block_subsidy / count_legacy_sigops ───────────────────────────────────────

pub(super) fn block_subsidy(height: BlockHeight) -> Satoshi {
    let halvings = height.0 / HALVING_INTERVAL;
    if halvings >= 64 {
        return Satoshi::ZERO;
    }
    Satoshi::new(INITIAL_SUBSIDY >> halvings)
}

/// Count legacy sigops (`OP_CHECKSIG` + `OP_CHECKSIGVERIFY` + `OP_CHECKMULTISIG` family)
/// in a raw script byte slice.
///
/// `OP_CHECKMULTISIG` / `OP_CHECKMULTISIGVERIFY` each count as 20 (the maximum
/// allowed pubkeys per multisig), matching Core's pre-segwit counting rule.
pub(super) fn count_legacy_sigops(script: &[u8]) -> u32 {
    const OP_CHECKSIG: u8 = 0xac;
    const OP_CHECKSIGVERIFY: u8 = 0xad;
    const OP_CHECKMULTISIG: u8 = 0xae;
    const OP_CHECKMULTISIGVERIFY: u8 = 0xaf;

    let mut count: u32 = 0;
    let mut i = 0;
    while i < script.len() {
        let op = script[i];
        match op {
            OP_CHECKSIG | OP_CHECKSIGVERIFY => count += 1,
            OP_CHECKMULTISIG | OP_CHECKMULTISIGVERIFY => count += 20,
            // Push data opcodes: skip the pushed bytes.
            0x01..=0x4b => i += op as usize,
            0x4c => {
                // OP_PUSHDATA1: next byte is length
                if let Some(&len) = script.get(i + 1) {
                    i += 1 + len as usize;
                } else {
                    break;
                }
            }
            0x4d => {
                // OP_PUSHDATA2: next 2 bytes are little-endian length
                if i + 2 < script.len() {
                    let len = u16::from_le_bytes([script[i + 1], script[i + 2]]) as usize;
                    i += 2 + len;
                } else {
                    break;
                }
            }
            0x4e => {
                // OP_PUSHDATA4: next 4 bytes are little-endian length
                if i + 4 < script.len() {
                    let len = u32::from_le_bytes([
                        script[i + 1],
                        script[i + 2],
                        script[i + 3],
                        script[i + 4],
                    ]) as usize;
                    i += 4 + len;
                } else {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    count
}
