//! Instruction construction and account reads.

use anchor_lang::InstructionData;
use market_tick::instruction::{Increment, Initialize};
use market_tick_abi::MarketTickV1;
use solana_commitment_config::CommitmentConfig;
use solana_signature::Signature;
use solana_transaction_error::TransactionError;
use std::{thread, time::Duration};

/// How long to wait between transaction status polls.
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How many times to poll before giving up on a submitted transaction.
const MAX_STATUS_POLLS: u32 = 200;

/// Submission settings used for slot-sensitive increments.
fn submit_config() -> RpcSendTransactionConfig {
    RpcSendTransactionConfig {
        skip_preflight: true,
        ..RpcSendTransactionConfig::default()
    }
}
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_rpc_client::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSendTransactionConfig;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::{
    config::{pda_v1, program_id},
    error::CliError,
};

/// Builds the `initialize` instruction for the canonical V1 PDA.
pub fn initialize_ix(payer: Pubkey) -> Instruction {
    Instruction {
        program_id: program_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(pda_v1().0, false),
            AccountMeta::new_readonly(solana_system_interface::program::id(), false),
        ],
        data: Initialize {}.data(),
    }
}

/// Builds an `increment` instruction signed by `signer`.
pub fn increment_ix(
    signer: Pubkey,
    slot: u64,
    timestamp_ns: i64,
    target_market_tick_interval_ns: u64,
) -> Instruction {
    Instruction {
        program_id: program_id(),
        accounts: vec![
            AccountMeta::new(pda_v1().0, false),
            AccountMeta::new_readonly(signer, true),
        ],
        data: Increment {
            slot,
            timestamp_ns,
            target_market_tick_interval_ns,
        }
        .data(),
    }
}

/// Signs and submits a single-instruction transaction, waiting for
/// confirmation at the client's commitment level.
pub fn send(
    rpc: &RpcClient,
    payer: &Keypair,
    ix: Instruction,
    extra_signers: &[&Keypair],
) -> Result<String, CliError> {
    let mut signers: Vec<&Keypair> = vec![payer];
    signers.extend_from_slice(extra_signers);

    let blockhash = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &signers, blockhash);
    Ok(rpc.send_and_confirm_transaction(&tx)?.to_string())
}

/// Submits an increment targeting the cluster's current slot.
///
/// The program requires the instruction slot to equal the runtime clock slot.
/// A client therefore has to guess which slot its transaction will execute in,
/// and any round trip spent preparing or landing the transaction can invalidate
/// that guess. This retries the resulting `SLOT_MISMATCH` a bounded number of
/// times, but success is inherently best effort from off-validator.
pub fn send_for_current_slot(
    rpc: &RpcClient,
    payer: &Keypair,
    timestamp_ns: i64,
    interval_ns: u64,
    attempts: u32,
) -> Result<String, CliError> {
    // A blockhash stays valid for many slots, so fetch it once instead of
    // paying for the round trip on every attempt.
    let blockhash = rpc.get_latest_blockhash()?;
    let mut last_error = None;

    for attempt in 0..attempts.max(1) {
        let slot =
            rpc.get_slot_with_commitment(CommitmentConfig::processed())? + u64::from(attempt);
        let ix = increment_ix(payer.pubkey(), slot, timestamp_ns, interval_ns);
        let tx =
            Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[payer], blockhash);

        // Skip preflight: simulation runs against the confirmed bank, which is
        // behind the bank that will execute this transaction, so it would
        // reject the very slot the program is about to accept. Submit without
        // blocking on confirmation so the transaction still lands in the slot
        // it targets, then resolve its real outcome below.
        let signature = match rpc.send_transaction_with_config(&tx, submit_config()) {
            Ok(signature) => signature,
            Err(error) => {
                let error = CliError::Rpc(error);
                if !is_slot_mismatch(&error) {
                    return Err(error);
                }
                last_error = Some(error);
                continue;
            }
        };

        // Never report success on submission alone: the program rejects a
        // stale slot at execution time, so the transaction can still fail.
        match await_execution(rpc, &signature)? {
            Ok(()) => return Ok(signature.to_string()),
            Err(error) => {
                if crate::error::transaction_custom_code(&error)
                    != Some(market_tick_abi::error_code::SLOT_MISMATCH)
                {
                    return Err(CliError::Executed(error));
                }
                last_error = Some(CliError::Executed(error));
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| CliError::Output("increment did not run for any slot".to_string())))
}

fn is_slot_mismatch(error: &CliError) -> bool {
    matches!(error, CliError::Rpc(rpc) if crate::error::custom_code(rpc)
        == Some(market_tick_abi::error_code::SLOT_MISMATCH))
}

/// Polls until the cluster reports whether a submitted transaction executed
/// successfully, returning the program's own error when it did not.
fn await_execution(
    rpc: &RpcClient,
    signature: &Signature,
) -> Result<Result<(), TransactionError>, CliError> {
    for _ in 0..MAX_STATUS_POLLS {
        if let Some(status) = rpc.get_signature_status(signature)? {
            return Ok(status);
        }
        thread::sleep(STATUS_POLL_INTERVAL);
    }

    Err(CliError::Output(format!(
        "timed out waiting for transaction {signature} to be processed"
    )))
}

/// Reads and decodes the canonical V1 account.
///
/// This verifies the program owner before decoding, matching the validation
/// sequence documented for consumers.
pub fn fetch_state(rpc: &RpcClient) -> Result<MarketTickV1, CliError> {
    let (pda, _) = pda_v1();
    let account = rpc
        .get_account_with_commitment(&pda, rpc.commitment())?
        .value
        .ok_or(CliError::NotInitialized(pda))?;

    if account.owner != program_id() {
        return Err(CliError::WrongOwner {
            account: pda,
            expected: program_id(),
            actual: account.owner,
        });
    }

    Ok(MarketTickV1::decode(&account.data)?)
}

/// Reports whether the canonical V1 account already exists and is
/// program-owned.
pub fn is_initialized(rpc: &RpcClient) -> Result<bool, CliError> {
    match fetch_state(rpc) {
        Ok(_) => Ok(true),
        Err(CliError::NotInitialized(_)) => Ok(false),
        Err(error) => Err(error),
    }
}
