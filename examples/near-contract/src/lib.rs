//! A protected application action executed in the same receipt as verification.

use gnorix::near::{Authorization, Commitment, EnrollmentView, contract::Enrollment};
use near_sdk::{
    AccountId, NearToken, PanicOnDefault, Promise, env, json_types::U64, near, store::LookupMap,
};

const EXAMPLE_SUITE: &str = "gnorix.example.hkdf-sha256-ed25519.v1";
const INCREMENT_OPERATION: &str = "counter.increment";
const MAX_INCREMENT: u64 = 100;

#[near(serializers = [borsh])]
struct AccountState {
    enrollment: Enrollment,
    counter: u64,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Counter {
    accounts: LookupMap<AccountId, AccountState>,
}

#[near]
impl Counter {
    #[init]
    pub fn new() -> Self {
        Self {
            accounts: LookupMap::new(b"accounts".as_slice()),
        }
    }

    /// Register the caller's own enrollment and pay for its actual storage.
    #[payable]
    pub fn register_enrollment(&mut self, commitment: Commitment) {
        let owner = commitment.owner_id.clone();
        assert!(
            self.accounts.get(&owner).is_none(),
            "owner already registered"
        );

        let enrollment = Enrollment::register(commitment, EXAMPLE_SUITE)
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));
        let storage_before = env::storage_usage();

        self.accounts.insert(
            owner,
            AccountState {
                enrollment,
                counter: 0,
            },
        );
        self.accounts.flush();
        settle_storage(storage_before);
    }

    /// Replace an existing enrollment with its next revision, preserving counter.
    #[payable]
    pub fn replace_enrollment(&mut self, commitment: Commitment) {
        let storage_before = env::storage_usage();
        let state = self.account_mut(&commitment.owner_id);

        state
            .enrollment
            .replace(commitment)
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));

        self.accounts.flush();
        settle_storage(storage_before);
    }

    pub fn revoke_enrollment(&mut self) {
        let owner = env::predecessor_account_id();
        self.account_mut(&owner)
            .enrollment
            .revoke()
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));
    }

    /// Any relayer may submit an authorization; the signed owner controls access.
    pub fn increment(
        &mut self,
        owner_id: AccountId,
        delta: U64,
        authorization: Authorization,
    ) -> U64 {
        let state = self.account_mut(&owner_id);
        state
            .enrollment
            .verify_and_consume(INCREMENT_OPERATION, &delta.0.to_be_bytes(), &authorization)
            .unwrap_or_else(|error| env::panic_str(&error.to_string()));

        // A later application failure must roll back nonce consumption as well.
        assert!(
            (1..=MAX_INCREMENT).contains(&delta.0),
            "delta must be within 1..=100"
        );
        let next = state
            .counter
            .checked_add(delta.0)
            .unwrap_or_else(|| env::panic_str("counter overflow"));
        state.counter = next;
        U64(next)
    }

    pub fn get_counter(&self, owner_id: AccountId) -> Option<U64> {
        self.accounts.get(&owner_id).map(|state| U64(state.counter))
    }

    pub fn get_enrollment(&self, owner_id: AccountId) -> Option<EnrollmentView> {
        self.accounts
            .get(&owner_id)
            .map(|state| state.enrollment.view())
    }

    /// Check arbitrary application data without consuming its nonce or applying
    /// the counter's permission rules. A successful view does not authorize a
    /// later transaction: that transaction must verify and consume again.
    pub fn verify_authorization(
        &self,
        owner_id: AccountId,
        operation: String,
        payload: Vec<u8>,
        authorization: Authorization,
    ) -> bool {
        self.accounts.get(&owner_id).is_some_and(|state| {
            state
                .enrollment
                .verify(&operation, &payload, &authorization)
                .is_ok()
        })
    }
}

impl Counter {
    fn account_mut(&mut self, owner: &AccountId) -> &mut AccountState {
        self.accounts
            .get_mut(owner)
            .unwrap_or_else(|| env::panic_str("owner is not registered"))
    }
}

/// Refunds only the current caller's excess deposit or released storage stake.
/// Failed verification/storage payment panics, rolling back the receipt's state.
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
