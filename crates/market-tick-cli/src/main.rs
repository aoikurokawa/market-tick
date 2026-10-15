//! Command line client for the Market Tick program.
//!
//! The CLI links the program crate directly, so instruction encoding, the
//! program ID, and the V1 PDA always match the deployed artifact without
//! requiring the Solana or Anchor CLIs.

mod client;
mod config;
mod error;
mod output;

use clap::{Parser, Subcommand};
use config::{ClusterArgs, KeypairArgs, OutputFormat};
use error::CliError;
use solana_signer::Signer;
use std::process::ExitCode;

/// How many times `increment` re-reads the clock and retries when the cluster
/// advances past the slot it targeted.
const INCREMENT_SLOT_ATTEMPTS: u32 = 5;

#[derive(Parser, Debug)]
#[command(
    name = "market-tick",
    about = "Client for the Market Tick program",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    cluster: ClusterArgs,

    #[command(flatten)]
    keypair: KeypairArgs,

    /// Output format.
    #[arg(long, value_name = "FORMAT", default_value = "text", global = true)]
    output: OutputFormat,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show the program ID and canonical V1 PDA without contacting a cluster.
    Address,

    /// Create the canonical V1 account.
    Initialize {
        /// Succeed quietly if the account already exists.
        #[arg(long)]
        skip_if_initialized: bool,
    },

    /// Read and decode the canonical V1 account.
    Show,

    /// Submit a tick for the current slot.
    Increment {
        /// Slot the tick applies to. Defaults to the cluster's current slot.
        #[arg(long, value_name = "SLOT")]
        slot: Option<u64>,

        /// POSIX timestamp in nanoseconds. Must be positive and, within a
        /// slot, strictly increasing.
        #[arg(long, value_name = "NANOS")]
        timestamp_ns: i64,

        /// Target tick interval in nanoseconds. Must be nonzero and constant
        /// within a slot.
        #[arg(long, value_name = "NANOS")]
        interval_ns: u64,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(rendered) => {
            if !rendered.is_empty() {
                println!("{rendered}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<String, CliError> {
    let (pda, bump) = config::pda_v1();

    match &cli.command {
        Command::Address => match cli.output {
            OutputFormat::Json => {
                let value = serde_json::json!({
                    "programId": config::program_id().to_string(),
                    "pdaV1": pda.to_string(),
                    "bump": bump,
                });
                serde_json::to_string_pretty(&value)
                    .map_err(|error| CliError::Output(error.to_string()))
            }
            OutputFormat::Text => Ok(format!(
                "program_id: {}\npda_v1:     {pda}\nbump:       {bump}",
                config::program_id()
            )),
        },

        Command::Show => {
            let rpc = cli.cluster.client();
            let state = client::fetch_state(&rpc)?;
            output::render_state(&state, &pda, cli.output)
        }

        Command::Initialize {
            skip_if_initialized,
        } => {
            let rpc = cli.cluster.client();
            if *skip_if_initialized && client::is_initialized(&rpc)? {
                let state = client::fetch_state(&rpc)?;
                return output::render_state(&state, &pda, cli.output);
            }

            let payer = cli.keypair.load()?;
            let signature = client::send(&rpc, &payer, client::initialize_ix(payer.pubkey()), &[])?;
            output::render_signature(&signature, "initialize", cli.output)
        }

        Command::Increment {
            slot,
            timestamp_ns,
            interval_ns,
        } => {
            let rpc = cli.cluster.client();
            let payer = cli.keypair.load()?;

            let signature = match slot {
                Some(slot) => client::send(
                    &rpc,
                    &payer,
                    client::increment_ix(payer.pubkey(), *slot, *timestamp_ns, *interval_ns),
                    &[],
                )?,
                // The program requires the instruction slot to equal the
                // runtime clock slot, but the cluster keeps advancing while the
                // transaction is prepared and lands. Re-read the slot and retry
                // a few times before giving up.
                None => client::send_for_current_slot(
                    &rpc,
                    &payer,
                    *timestamp_ns,
                    *interval_ns,
                    INCREMENT_SLOT_ATTEMPTS,
                )?,
            };
            output::render_signature(&signature, "increment", cli.output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn address_command_needs_no_cluster_flags() {
        let cli = Cli::try_parse_from(["market-tick", "address"]).expect("parse");
        assert!(matches!(cli.command, Command::Address));
        assert_eq!(cli.output, OutputFormat::Text);
    }

    #[test]
    fn monikers_resolve_to_known_endpoints() {
        let cli = Cli::try_parse_from(["market-tick", "--url", "devnet", "show"]).expect("parse");
        assert_eq!(cli.cluster.resolved_url(), "https://api.devnet.solana.com");

        let cli = Cli::try_parse_from(["market-tick", "--url", "localnet", "show"]).expect("parse");
        assert_eq!(cli.cluster.resolved_url(), "http://127.0.0.1:8899");

        let cli = Cli::try_parse_from(["market-tick", "--url", "https://example.com", "show"])
            .expect("parse");
        assert_eq!(cli.cluster.resolved_url(), "https://example.com");
    }

    #[test]
    fn increment_requires_timestamp_and_interval() {
        assert!(Cli::try_parse_from(["market-tick", "increment"]).is_err());
        assert!(
            Cli::try_parse_from(["market-tick", "increment", "--timestamp-ns", "1000"]).is_err()
        );

        let cli = Cli::try_parse_from([
            "market-tick",
            "increment",
            "--timestamp-ns",
            "1000",
            "--interval-ns",
            "50",
        ])
        .expect("parse");

        match cli.command {
            Command::Increment {
                slot,
                timestamp_ns,
                interval_ns,
            } => {
                assert_eq!(slot, None);
                assert_eq!(timestamp_ns, 1_000);
                assert_eq!(interval_ns, 50);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn json_output_is_selectable() {
        let cli = Cli::try_parse_from(["market-tick", "show", "--output", "json"]).expect("parse");
        assert_eq!(cli.output, OutputFormat::Json);
    }
}
