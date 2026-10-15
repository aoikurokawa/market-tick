//! Error type shared by all subcommands.

use market_tick_abi::{error_code, AbiError};
use solana_instruction::error::InstructionError;
use solana_pubkey::Pubkey;
use solana_rpc_client::api::client_error::Error as ClientError;
use solana_transaction_error::TransactionError;
use std::fmt;

/// Extracts a program-defined error number from a failed transaction.
pub(crate) fn custom_code(error: &ClientError) -> Option<u32> {
    error
        .get_transaction_error()
        .as_ref()
        .and_then(transaction_custom_code)
}

/// Extracts a program-defined error number from an executed transaction's
/// failure.
pub(crate) fn transaction_custom_code(error: &TransactionError) -> Option<u32> {
    match error {
        TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
        _ => None,
    }
}

/// Maps a Market Tick custom error number to its name and an explanation of
/// what the program rejected.
fn describe_program_error(code: u32) -> Option<(&'static str, &'static str)> {
    match code {
        error_code::SLOT_MISMATCH => Some((
            "SLOT_MISMATCH",
            "the instruction slot did not equal the runtime clock slot; the \
             cluster advanced before the transaction landed",
        )),
        error_code::NOT_SLOT_SIGNER => Some((
            "NOT_SLOT_SIGNER",
            "another key already claimed this slot; only that signer may submit \
             further ticks until the next slot",
        )),
        error_code::NON_MONOTONIC => Some((
            "NON_MONOTONIC",
            "the timestamp was not strictly greater than the previous one in \
             this slot",
        )),
        error_code::INTERVAL_MISMATCH => Some((
            "INTERVAL_MISMATCH",
            "the target interval differs from the one selected for this slot",
        )),
        error_code::COUNTER_OVERFLOW => Some((
            "COUNTER_OVERFLOW",
            "the same-slot sequence counter would overflow",
        )),
        error_code::INVALID_TIMESTAMP => {
            Some(("INVALID_TIMESTAMP", "the timestamp must be positive"))
        }
        error_code::INVALID_INTERVAL => {
            Some(("INVALID_INTERVAL", "the target interval must be nonzero"))
        }
        _ => None,
    }
}

/// Any failure surfaced by the CLI.
#[derive(Debug)]
pub enum CliError {
    /// A local keypair could not be loaded.
    Keypair(String),
    /// An RPC request failed.
    Rpc(ClientError),
    /// The transaction was accepted by the cluster but failed during
    /// execution.
    Executed(TransactionError),
    /// The account bytes could not be decoded as the V1 ABI.
    Abi(AbiError),
    /// The account exists but is not owned by the Market Tick program.
    WrongOwner {
        /// Address that was inspected.
        account: Pubkey,
        /// Owner the CLI expected.
        expected: Pubkey,
        /// Owner actually reported by the cluster.
        actual: Pubkey,
    },
    /// The canonical PDA does not exist yet.
    NotInitialized(Pubkey),
    /// Serializing JSON output failed.
    Output(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keypair(message) => write!(f, "{message}"),
            Self::Rpc(error) => match custom_code(error).and_then(describe_program_error) {
                Some((name, explanation)) => {
                    write!(
                        f,
                        "program rejected the transaction: {name} ({explanation})"
                    )
                }
                None => write!(f, "RPC request failed: {error}"),
            },
            Self::Executed(error) => {
                match transaction_custom_code(error).and_then(describe_program_error) {
                    Some((name, explanation)) => write!(
                        f,
                        "program rejected the transaction: {name} ({explanation})"
                    ),
                    None => write!(f, "transaction failed during execution: {error}"),
                }
            }
            Self::Abi(error) => write!(f, "account did not match the V1 ABI: {error:?}"),
            Self::WrongOwner {
                account,
                expected,
                actual,
            } => write!(
                f,
                "account {account} is owned by {actual}, expected {expected}"
            ),
            Self::NotInitialized(pda) => write!(
                f,
                "the V1 account {pda} does not exist; run `market-tick initialize` first"
            ),
            Self::Output(message) => write!(f, "failed to render output: {message}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<ClientError> for CliError {
    fn from(error: ClientError) -> Self {
        Self::Rpc(error)
    }
}

impl From<AbiError> for CliError {
    fn from(error: AbiError) -> Self {
        Self::Abi(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_abi_error_code_has_a_description() {
        for code in [
            error_code::SLOT_MISMATCH,
            error_code::NOT_SLOT_SIGNER,
            error_code::NON_MONOTONIC,
            error_code::INTERVAL_MISMATCH,
            error_code::COUNTER_OVERFLOW,
            error_code::INVALID_TIMESTAMP,
            error_code::INVALID_INTERVAL,
        ] {
            assert!(
                describe_program_error(code).is_some(),
                "missing description for error {code}"
            );
        }
    }

    #[test]
    fn unknown_error_codes_are_not_described() {
        assert!(describe_program_error(1).is_none());
        assert!(describe_program_error(7000).is_none());
    }

    #[test]
    fn not_initialized_message_suggests_initialize() {
        let message = CliError::NotInitialized(Pubkey::new_from_array([1u8; 32])).to_string();
        assert!(message.contains("market-tick initialize"), "{message}");
    }
}
