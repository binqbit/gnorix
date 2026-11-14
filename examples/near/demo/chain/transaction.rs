//! Public transaction observations interpreted from finalized chain receipts.

use gnorix_near_wallet::{TransferOutcome, TransferStatus};
use near_primitives::{
    hash::CryptoHash,
    views::{ExecutionStatusView, FinalExecutionOutcomeView, FinalExecutionStatus},
};
use near_sdk::serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(crate = "near_sdk::serde", rename_all = "snake_case")]
pub enum SubmissionStatus {
    Succeeded,
    Failed,
    Rejected,
    Pending,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ReceiptFailure {
    pub receipt_id: String,
    pub executor_id: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TransferResult {
    pub wallet_index: usize,
    pub revision: String,
    pub nonce: String,
    pub tx_hash: Option<String>,
    pub relayer_id: String,
    pub status: SubmissionStatus,
    pub callback: Option<TransferOutcome>,
    pub gas_burnt: Option<String>,
    pub failures: Vec<ReceiptFailure>,
    pub logs: Vec<String>,
    pub balances: Option<[String; 2]>,
    pub observation_error: Option<String>,
}

pub(super) struct PendingTransfer {
    pub(super) hash: CryptoHash,
    pub(super) result: TransferResult,
}

pub(super) fn summarize_execution(
    outcome: &FinalExecutionOutcomeView,
    pending: &PendingTransfer,
) -> TransferResult {
    let mut result = pending.result.clone();
    if outcome.transaction.hash != pending.hash {
        result.status = SubmissionStatus::Unknown;
        result.observation_error = Some("RPC returned a different transaction outcome".into());
        return result;
    }
    result.observation_error = None;
    let all = std::iter::once(&outcome.transaction_outcome).chain(&outcome.receipts_outcome);
    let mut gas = 0_u128;
    result.failures.clear();
    result.logs.clear();
    for receipt in all {
        gas += u128::from(receipt.outcome.gas_burnt.as_gas());
        result.logs.extend(receipt.outcome.logs.iter().cloned());
        if let ExecutionStatusView::Failure(error) = &receipt.outcome.status {
            result.failures.push(ReceiptFailure {
                receipt_id: receipt.id.to_string(),
                executor_id: receipt.outcome.executor_id.to_string(),
                error: error.to_string(),
            });
        }
    }
    result.gas_burnt = Some(gas.to_string());
    match &outcome.status {
        FinalExecutionStatus::SuccessValue(value) => {
            match serde_json::from_slice::<TransferOutcome>(value) {
                Ok(callback)
                    if callback.revision.0.to_string() == result.revision
                        && callback.nonce.0.to_string() == result.nonce =>
                {
                    result.status = match callback.status {
                        TransferStatus::Succeeded => SubmissionStatus::Succeeded,
                        TransferStatus::Failed => SubmissionStatus::Failed,
                        _ => SubmissionStatus::Unknown,
                    };
                    result.callback = Some(callback);
                }
                _ => {
                    result.status = SubmissionStatus::Unknown;
                    result.observation_error =
                        Some("wallet returned an unexpected callback result".into());
                }
            }
        }
        FinalExecutionStatus::Failure(error) => {
            result.status = if acceptance_rejected(outcome) {
                SubmissionStatus::Rejected
            } else {
                SubmissionStatus::Unknown
            };
            result.observation_error = Some(format!("transaction execution failed: {error}"));
        }
        _ => result.status = SubmissionStatus::Pending,
    }
    result
}

fn acceptance_rejected(outcome: &FinalExecutionOutcomeView) -> bool {
    if matches!(
        outcome.transaction_outcome.outcome.status,
        ExecutionStatusView::Failure(_)
    ) {
        return true;
    }
    outcome.receipts_outcome.iter().any(|receipt| {
        outcome
            .transaction_outcome
            .outcome
            .receipt_ids
            .contains(&receipt.id)
            && matches!(receipt.outcome.status, ExecutionStatusView::Failure(_))
    })
}
