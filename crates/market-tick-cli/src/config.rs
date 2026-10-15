//! Shared connection, signer, and output options.

use clap::Args;
use solana_commitment_config::CommitmentConfig;
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_rpc_client::rpc_client::RpcClient;
use std::{path::PathBuf, str::FromStr};

use crate::error::CliError;

/// Options describing which cluster to talk to.
#[derive(Args, Debug, Clone)]
pub struct ClusterArgs {
    /// RPC endpoint, or one of the aliases `localnet`, `devnet`, `testnet`,
    /// `mainnet`.
    #[arg(
        long,
        short = 'u',
        value_name = "URL_OR_MONIKER",
        env = "ANCHOR_PROVIDER_URL",
        default_value = "localnet",
        global = true
    )]
    pub url: String,

    /// Commitment level used for reads and confirmations.
    #[arg(
        long,
        value_name = "LEVEL",
        value_parser = ["processed", "confirmed", "finalized"],
        default_value = "confirmed",
        global = true
    )]
    pub commitment: String,
}

impl ClusterArgs {
    /// Resolves cluster monikers to concrete RPC URLs.
    pub fn resolved_url(&self) -> String {
        match self.url.as_str() {
            "localnet" | "localhost" => "http://127.0.0.1:8899".to_string(),
            "devnet" => "https://api.devnet.solana.com".to_string(),
            "testnet" => "https://api.testnet.solana.com".to_string(),
            "mainnet" | "mainnet-beta" => "https://api.mainnet-beta.solana.com".to_string(),
            other => other.to_string(),
        }
    }

    fn commitment(&self) -> CommitmentConfig {
        match self.commitment.as_str() {
            "processed" => CommitmentConfig::processed(),
            "finalized" => CommitmentConfig::finalized(),
            _ => CommitmentConfig::confirmed(),
        }
    }

    /// Builds an RPC client for the resolved endpoint.
    pub fn client(&self) -> RpcClient {
        RpcClient::new_with_commitment(self.resolved_url(), self.commitment())
    }
}

/// Options describing which local keypair signs and pays.
#[derive(Args, Debug, Clone)]
pub struct KeypairArgs {
    /// Path to the fee-payer keypair file.
    #[arg(
        long,
        short = 'k',
        value_name = "PATH",
        env = "ANCHOR_WALLET",
        global = true
    )]
    pub keypair: Option<PathBuf>,
}

impl KeypairArgs {
    /// Loads the configured keypair, falling back to the Solana CLI default
    /// location at `~/.config/solana/id.json`.
    pub fn load(&self) -> Result<Keypair, CliError> {
        let path = match &self.keypair {
            Some(path) => path.clone(),
            None => default_keypair_path()?,
        };

        read_keypair_file(&path).map_err(|error| {
            CliError::Keypair(format!(
                "failed to read keypair from {}: {error}",
                path.display()
            ))
        })
    }
}

fn default_keypair_path() -> Result<PathBuf, CliError> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        CliError::Keypair("no keypair given and HOME is unset; pass --keypair".to_string())
    })?;
    Ok(PathBuf::from(home).join(".config/solana/id.json"))
}

/// How command results should be rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable aligned text.
    Text,
    /// Machine-readable JSON.
    Json,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!("unknown output format `{other}`")),
        }
    }
}

/// Derives the canonical V1 PDA and bump for the compiled-in program ID.
pub fn pda_v1() -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            market_tick_abi::PDA_SEED_MARKET_TICK,
            market_tick_abi::PDA_SEED_VERSION_V1,
        ],
        &program_id(),
    )
}

/// Program ID this CLI was compiled against.
pub fn program_id() -> Pubkey {
    market_tick::id()
}
