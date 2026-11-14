use anyhow::{Result, bail, ensure};
use gnorix::near::Authorization;
use gnorix_example_support::fingerprint::Circle;
use serde::{Deserialize, Serialize};

use super::{
    chain::TransferResult,
    fingerprint::{FeaturePoint, FingerprintView},
};

pub const TOKEN_DECIMALS: u32 = 24;
pub const TOKEN_UNIT: u128 = 10_u128.pow(TOKEN_DECIMALS);

#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub token: String,
    pub phase: &'static str,
    pub message: String,
    pub error: Option<String>,
    pub busy: bool,
    pub chain_id: Option<String>,
    pub token_id: Option<String>,
    pub symbol: &'static str,
    pub decimals: u32,
    pub wallets: Vec<WalletView>,
    pub prepared: Option<PreparedView>,
    pub capture: Option<CaptureView>,
    pub transaction: Option<TransferResult>,
}

#[derive(Clone, Serialize)]
pub struct WalletView {
    pub id: usize,
    pub name: String,
    pub account_id: String,
    pub balance_base_units: String,
    pub root: String,
    pub nonce: String,
    pub leaf_count: u64,
    pub fingerprint: FingerprintView,
}

#[derive(Clone, Serialize)]
pub struct PreparedView {
    pub id: String,
    pub capture_id: String,
    pub wallet: usize,
    pub recipient: usize,
    pub amount: String,
    pub amount_base_units: String,
    pub memo: Option<String>,
    pub expires_at_ms: String,
    pub authorization: Authorization,
    pub matched_feature_ids: Vec<usize>,
    pub attempts: usize,
    pub verified: bool,
}

#[derive(Clone, Serialize)]
pub struct CaptureView {
    pub id: String,
    pub wallet: usize,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub region: Circle,
    pub pattern_fraction: f64,
    pub feature_count: Option<usize>,
    pub points: Vec<FeaturePoint>,
    pub matched_feature_ids: Vec<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignRequest {
    pub wallet: usize,
    pub recipient: usize,
    pub amount: String,
    pub memo: Option<String>,
}

impl SignRequest {
    pub fn validate(&self) -> Result<u128> {
        ensure!(
            self.wallet < 2 && self.recipient == 1 - self.wallet,
            "select the other demo wallet as recipient"
        );
        ensure!(
            self.memo.as_ref().is_none_or(|memo| memo.len() <= 256),
            "memo exceeds 256 UTF-8 bytes"
        );
        parse_amount(&self.amount)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitRequest {
    pub prepared_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {}

/// Parse a human token amount exactly; never round or pass through floating point.
pub fn parse_amount(value: &str) -> Result<u128> {
    ensure!(value.len() <= 64, "amount is too long");
    let (whole, fraction) = value
        .split_once('.')
        .map_or((value, None), |(whole, fraction)| (whole, Some(fraction)));
    ensure!(
        !whole.is_empty() && whole.bytes().all(|b| b.is_ascii_digit()),
        "amount must be a positive decimal number"
    );
    let whole: u128 = whole
        .parse()
        .map_err(|_| anyhow::anyhow!("amount is too large"))?;
    let mut units = whole
        .checked_mul(TOKEN_UNIT)
        .ok_or_else(|| anyhow::anyhow!("amount is too large"))?;
    if let Some(fraction) = fraction {
        ensure!(
            !fraction.is_empty()
                && fraction.len() <= TOKEN_DECIMALS as usize
                && fraction.bytes().all(|b| b.is_ascii_digit()),
            "amount allows at most 24 decimal places"
        );
        let value: u128 = fraction.parse()?;
        let scaled = value * 10_u128.pow(TOKEN_DECIMALS - fraction.len() as u32);
        units = units
            .checked_add(scaled)
            .ok_or_else(|| anyhow::anyhow!("amount is too large"))?;
    }
    if units == 0 {
        bail!("amount must be greater than zero");
    }
    Ok(units)
}

pub fn format_amount(units: u128) -> String {
    let whole = units / TOKEN_UNIT;
    let remainder = units % TOKEN_UNIT;
    if remainder == 0 {
        whole.to_string()
    } else {
        let fraction = format!("{remainder:024}");
        format!("{whole}.{}", fraction.trim_end_matches('0'))
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_preserve_the_smallest_unit_and_reject_rounding_or_overflow() {
        for units in [
            1,
            TOKEN_UNIT - 1,
            TOKEN_UNIT,
            125 * TOKEN_UNIT + 1,
            u128::MAX,
        ] {
            assert_eq!(parse_amount(&format_amount(units)).unwrap(), units);
        }
        assert_eq!(
            parse_amount("00012.5000").unwrap(),
            12 * TOKEN_UNIT + TOKEN_UNIT / 2
        );
        for value in [
            "",
            "0",
            "-1",
            "+1",
            "1e3",
            "1.",
            ".5",
            " 1",
            "NaN",
            "1.2.3",
            "0.0000000000000000000000001",
            "340282366920939",
        ] {
            assert!(parse_amount(value).is_err(), "accepted {value}");
        }
    }
}
