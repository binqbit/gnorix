//! An ordinary NEP-141 token with NEP-145 storage and NEP-148 metadata.
//!
//! The initial supply belongs to the chosen owner. A smart wallet can hold and
//! transfer this token as any other NEAR account; authorization stays outside
//! this contract.

use near_contract_standards::{
    fungible_token::{
        FungibleToken, FungibleTokenCore, FungibleTokenResolver,
        events::{FtBurn, FtMint},
        metadata::{FT_METADATA_SPEC, FungibleTokenMetadata, FungibleTokenMetadataProvider},
    },
    storage_management::{StorageBalance, StorageBalanceBounds, StorageManagement},
};
use near_sdk::{AccountId, NearToken, PanicOnDefault, PromiseOrValue, json_types::U128, near};

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Token {
    token: FungibleToken,
}

#[near]
impl Token {
    /// Deploy and initialize from the contract account, minting the only initial
    /// supply to `owner_id`. There is no subsequent minting authority.
    #[private]
    #[init]
    pub fn new(owner_id: AccountId, total_supply: U128) -> Self {
        Self {
            token: create_token(&owner_id, total_supply),
        }
    }
}

#[near]
impl FungibleTokenCore for Token {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        self.token.ft_transfer(receiver_id, amount, memo);
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        self.token.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        self.token.ft_total_supply()
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        self.token.ft_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenResolver for Token {
    #[private]
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        self.token
            .ft_resolve_transfer(sender_id, receiver_id, amount)
    }
}

#[near]
impl StorageManagement for Token {
    #[payable]
    fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        let Some((account_id, balance)) = self.token.internal_storage_unregister(force) else {
            return false;
        };

        if balance > 0 {
            FtBurn {
                owner_id: &account_id,
                amount: U128(balance),
                memo: Some("storage unregister"),
            }
            .emit();
        }

        true
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenMetadataProvider for Token {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_owned(),
            name: "Gnorix Test Token".to_owned(),
            symbol: "GXT".to_owned(),
            decimals: 24,
            icon: None,
            reference: None,
            reference_hash: None,
        }
    }
}

fn create_token(owner_id: &AccountId, total_supply: U128) -> FungibleToken {
    let mut token = FungibleToken::new(b"balances".as_slice());
    token.internal_register_account(owner_id);
    token.internal_deposit(owner_id, total_supply.0);

    FtMint {
        owner_id,
        amount: total_supply,
        memo: Some("initial supply"),
    }
    .emit();

    token
}
