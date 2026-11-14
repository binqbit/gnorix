//! A single-owner wallet that authorizes standard NEP-141 transfers with Gnorix.
//! Tokens belong to this contract's account, not to an external owner's account.

mod transfer;

pub use transfer::{TRANSFER_OPERATION, Transfer};

use gnorix::near::{Authorization, Commitment, EnrollmentView, contract::Enrollment};
use near_sdk::{
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError, env, json_types::U64, near,
    require, serde_json::json, store::LazyOption,
};

const EXAMPLE_SUITE: &str = "gnorix.example.hkdf-sha256-ed25519.v1";
const TRANSFER_GAS: Gas = Gas::from_tgas(20);
const CALLBACK_GAS: Gas = Gas::from_tgas(10);
const SCHEDULE_MARGIN: Gas = Gas::from_tgas(5);
const MAX_MEMO_BYTES: usize = 256;

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Idle,
    Pending,
    Succeeded,
    Failed,
}

/// Latest accepted transfer only; fixed-size state prevents relayer-driven growth.
/// Success means that the signed token contract's receipt succeeded.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferOutcome {
    pub revision: U64,
    pub nonce: U64,
    pub status: TransferStatus,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Wallet {
    owner_id: AccountId,
    enrollment: LazyOption<Enrollment>,
    latest_transfer: TransferOutcome,
}

#[near]
impl Wallet {
    /// Initialize in a deployment transaction signed by the wallet account.
    /// The owner can be this same account or a separate management account.
    #[init]
    #[private]
    pub fn new(owner_id: AccountId) -> Self {
        Self {
            owner_id,
            enrollment: LazyOption::new(b"enrollment".as_slice(), None),
            latest_transfer: TransferOutcome {
                revision: U64(0),
                nonce: U64(0),
                status: TransferStatus::Idle,
            },
        }
    }

    #[payable]
    pub fn register_enrollment(&mut self, commitment: Commitment) {
        self.require_owner();
        require!(self.enrollment.get().is_none(), "owner already registered");
        require!(
            commitment.owner_id == self.owner_id,
            "wrong enrollment owner"
        );

        let record = Enrollment::register(commitment, EXAMPLE_SUITE)
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));
        let storage_before = env::storage_usage();
        self.enrollment.set(Some(record));
        self.enrollment.flush();
        settle_storage(storage_before);
    }

    #[payable]
    pub fn replace_enrollment(&mut self, commitment: Commitment) {
        self.require_owner();
        self.require_no_pending_transfer();
        let storage_before = env::storage_usage();
        self.enrollment_mut()
            .replace(commitment)
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));

        self.enrollment.flush();
        settle_storage(storage_before);
    }

    /// Prevent new authorizations; an already dispatched transfer cannot be undone.
    pub fn revoke_enrollment(&mut self) {
        self.require_owner();
        self.enrollment_mut()
            .revoke()
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));
    }

    pub fn get_owner(&self) -> AccountId {
        self.owner_id.clone()
    }

    pub fn get_enrollment(&self) -> Option<EnrollmentView> {
        self.enrollment.get().as_ref().map(Enrollment::view)
    }

    pub fn get_transfer_status(&self) -> TransferOutcome {
        self.latest_transfer.clone()
    }

    /// Read-only cryptographic check. Does not reserve a nonce or promise that
    /// token balances, storage registrations or gas will allow execution.
    pub fn verify_transfer(&self, transfer: Transfer, authorization: Authorization) -> bool {
        self.enrollment.get().as_ref().is_some_and(|record| {
            record
                .verify(
                    TRANSFER_OPERATION,
                    &transfer.signing_payload(),
                    &authorization,
                )
                .is_ok()
        })
    }

    /// Any relayer may submit the approved transfer with zero attached deposit.
    /// The wallet funds the standard token call's one yoctoNEAR deposit.
    pub fn transfer(&mut self, transfer: Transfer, authorization: Authorization) -> Promise {
        self.require_no_pending_transfer();
        validate_transfer(&transfer);
        self.enrollment_mut()
            .verify_and_consume(
                TRANSFER_OPERATION,
                &transfer.signing_payload(),
                &authorization,
            )
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));
        require_scheduling_gas();

        self.latest_transfer = TransferOutcome {
            revision: U64(authorization.revision),
            nonce: U64(authorization.action.nonce),
            status: TransferStatus::Pending,
        };
        self.dispatch_transfer(transfer)
    }

    /// Callback is private, matches the accepted request, and never restores a
    /// nonce. A failed external receipt cannot roll back the accepting receipt.
    #[private]
    pub fn on_transfer(&mut self, revision: U64, nonce: U64) -> TransferOutcome {
        require!(
            env::promise_results_count() == 1,
            "expected one transfer result"
        );
        require!(
            self.latest_transfer.status == TransferStatus::Pending
                && self.latest_transfer.revision == revision
                && self.latest_transfer.nonce == nonce,
            "callback does not match the pending transfer"
        );

        // Only receipt success matters. Never allocate untrusted return data.
        // TooLong means the receipt succeeded but returned more than zero bytes.
        self.latest_transfer.status = receipt_status(env::promise_result_checked(0, 0));
        self.latest_transfer.clone()
    }
}

impl Wallet {
    fn require_owner(&self) {
        require!(
            env::predecessor_account_id() == self.owner_id,
            "only the wallet owner"
        );
    }

    fn require_no_pending_transfer(&self) {
        require!(
            self.latest_transfer.status != TransferStatus::Pending,
            "a transfer is still pending"
        );
    }

    fn enrollment_mut(&mut self) -> &mut Enrollment {
        self.enrollment
            .get_mut()
            .as_mut()
            .unwrap_or_else(|| env::panic_str("owner is not registered"))
    }

    fn dispatch_transfer(&self, transfer: Transfer) -> Promise {
        let args = json!({
            "receiver_id": transfer.receiver_id,
            "amount": transfer.amount,
            "memo": transfer.memo,
        });
        let callback_args = json!({
            "revision": self.latest_transfer.revision,
            "nonce": self.latest_transfer.nonce,
        });

        Promise::new(transfer.token_id)
            .function_call(
                "ft_transfer",
                args.to_string().into_bytes(),
                NearToken::from_yoctonear(1),
                TRANSFER_GAS,
            )
            .then(Promise::new(env::current_account_id()).function_call(
                "on_transfer",
                callback_args.to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                CALLBACK_GAS,
            ))
    }
}

fn validate_transfer(transfer: &Transfer) {
    require!(transfer.amount.0 > 0, "amount must be positive");
    require!(
        transfer.receiver_id != env::current_account_id(),
        "cannot transfer to self"
    );
    require!(
        transfer.token_id != env::current_account_id(),
        "token must be another contract"
    );
    require!(
        transfer
            .memo
            .as_ref()
            .is_none_or(|memo| memo.len() <= MAX_MEMO_BYTES),
        "memo exceeds 256 bytes"
    );
}

fn require_scheduling_gas() {
    let available = env::prepaid_gas()
        .as_gas()
        .saturating_sub(env::used_gas().as_gas());
    let required = TRANSFER_GAS.as_gas() + CALLBACK_GAS.as_gas() + SCHEDULE_MARGIN.as_gas();
    require!(
        available >= required,
        "insufficient gas for transfer and callback"
    );
}

fn receipt_status(result: Result<Vec<u8>, PromiseError>) -> TransferStatus {
    match result {
        Ok(_) | Err(PromiseError::TooLong(_)) => TransferStatus::Succeeded,
        Err(_) => TransferStatus::Failed,
    }
}

fn settle_storage(storage_before: u64) {
    let storage_after = env::storage_usage();
    let byte_cost = env::storage_byte_cost().as_yoctonear();
    let attached = env::attached_deposit().as_yoctonear();
    let refund = if storage_after >= storage_before {
        let cost = u128::from(storage_after - storage_before) * byte_cost;
        attached
            .checked_sub(cost)
            .unwrap_or_else(|| env::panic_str("insufficient storage deposit"))
    } else {
        attached + u128::from(storage_before - storage_after) * byte_cost
    };

    if refund > 0 {
        let _refund =
            Promise::new(env::predecessor_account_id()).transfer(NearToken::from_yoctonear(refund));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_tracks_receipt_success_without_trusting_return_data() {
        assert_eq!(receipt_status(Ok(vec![])), TransferStatus::Succeeded);
        assert_eq!(
            receipt_status(Err(PromiseError::TooLong(usize::MAX))),
            TransferStatus::Succeeded
        );
        assert_eq!(
            receipt_status(Err(PromiseError::Failed)),
            TransferStatus::Failed
        );
    }
}
