//! Test-only receiver for standard NEP-141 transfer/callback scenarios.
//! Its message-selected behavior is a fixture, not application authorization.

use near_sdk::{AccountId, PanicOnDefault, env, json_types::U128, near, require};

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Receiver {
    token_id: AccountId,
}

#[near]
impl Receiver {
    #[init]
    pub fn new(token_id: AccountId) -> Self {
        Self { token_id }
    }

    pub fn ft_on_transfer(&self, sender_id: AccountId, amount: U128, msg: String) -> U128 {
        require!(
            env::predecessor_account_id() == self.token_id,
            "only the configured token may invoke this receiver"
        );
        let _ = sender_id;

        match msg.as_str() {
            "keep" => U128(0),
            "refund_half" => U128(amount.0 / 2),
            "refund_all" => amount,
            "reject" => env::panic_str("receiver rejected transfer"),
            _ => env::panic_str("unknown test receiver behavior"),
        }
    }
}
