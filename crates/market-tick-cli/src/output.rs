//! Rendering of account state in text and JSON form.

use market_tick_abi::MarketTickV1;
use solana_pubkey::Pubkey;

use crate::{config::OutputFormat, error::CliError};

/// Renders decoded V1 state.
pub fn render_state(
    state: &MarketTickV1,
    pda: &Pubkey,
    format: OutputFormat,
) -> Result<String, CliError> {
    let signer = Pubkey::new_from_array(state.signer);
    // An all-zero signer means no tick has been accepted yet, which is
    // distinct from a tick signed by the system program.
    let has_tick = state.signer != [0u8; 32];

    match format {
        OutputFormat::Json => {
            let value = serde_json::json!({
                "address": pda.to_string(),
                "hasTick": has_tick,
                "signer": has_tick.then(|| signer.to_string()),
                "slot": state.slot,
                "firstTimestampNs": state.first_timestamp_ns,
                "timestampNs": state.timestamp_ns,
                "sequence": state.sequence,
                "targetMarketTickIntervalNs": state.target_market_tick_interval_ns,
                "observedMarketTickIntervalNs": state.observed_market_tick_interval_ns,
            });
            serde_json::to_string_pretty(&value)
                .map_err(|error| CliError::Output(error.to_string()))
        }
        OutputFormat::Text => {
            let signer_display = if has_tick {
                signer.to_string()
            } else {
                "<none, no tick accepted yet>".to_string()
            };

            Ok(format!(
                "address:                {pda}\n\
                 signer:                 {signer_display}\n\
                 slot:                   {}\n\
                 first_timestamp_ns:     {}\n\
                 timestamp_ns:           {}\n\
                 sequence:               {}\n\
                 target_interval_ns:     {}\n\
                 observed_interval_ns:   {}",
                state.slot,
                state.first_timestamp_ns,
                state.timestamp_ns,
                state.sequence,
                state.target_market_tick_interval_ns,
                state.observed_market_tick_interval_ns,
            ))
        }
    }
}

/// Renders the result of a state-changing transaction.
pub fn render_signature(
    signature: &str,
    action: &str,
    format: OutputFormat,
) -> Result<String, CliError> {
    match format {
        OutputFormat::Json => {
            let value = serde_json::json!({ "action": action, "signature": signature });
            serde_json::to_string_pretty(&value)
                .map_err(|error| CliError::Output(error.to_string()))
        }
        OutputFormat::Text => Ok(format!("{action}: {signature}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pda() -> Pubkey {
        Pubkey::new_from_array([7u8; 32])
    }

    #[test]
    fn default_state_reports_no_tick() {
        let rendered =
            render_state(&MarketTickV1::default(), &pda(), OutputFormat::Text).expect("render");
        assert!(rendered.contains("<none, no tick accepted yet>"));
    }

    #[test]
    fn json_marks_absent_signer_as_null() {
        let rendered =
            render_state(&MarketTickV1::default(), &pda(), OutputFormat::Json).expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!(value["hasTick"], serde_json::Value::Bool(false));
        assert_eq!(value["signer"], serde_json::Value::Null);
    }

    #[test]
    fn json_reports_populated_state() {
        let state = MarketTickV1 {
            signer: [3u8; 32],
            slot: 42,
            first_timestamp_ns: 1_000,
            timestamp_ns: 1_050,
            sequence: 1,
            target_market_tick_interval_ns: 50,
            observed_market_tick_interval_ns: 50,
        };
        let rendered = render_state(&state, &pda(), OutputFormat::Json).expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");

        assert_eq!(value["hasTick"], serde_json::Value::Bool(true));
        assert_eq!(
            value["signer"],
            Pubkey::new_from_array([3u8; 32]).to_string()
        );
        assert_eq!(value["slot"], 42);
        assert_eq!(value["sequence"], 1);
        assert_eq!(value["observedMarketTickIntervalNs"], 50);
    }
}
