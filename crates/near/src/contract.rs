//! Persistent authorization state for a protected function in the same contract.
//!
//! Store one [`Enrollment`] per owner in the application's NEAR storage. Owner
//! lifecycle methods use the immediate predecessor, while signed actions may be
//! relayed by any account. The application must persist the mutated record and
//! execute its action in the same receipt. This API does not authorize promises
//! or roll back an earlier receipt when an asynchronous call fails.

use borsh::{BorshDeserialize, BorshSerialize};
use ed25519_dalek::{Signature, VerifyingKey};
use gnorix_core::{
    authorization::{self, EnrollmentStatus},
    crypto::SignatureVerifier,
    error::AlgorithmError,
};
use near_sdk::env;

use crate::{
    AccountId, Action, Authorization, Commitment, EnrollmentView, Error, Result,
    verification_limits,
};

/// Ed25519 verification with the same strict acceptance rules as the offchain
/// example provider. `suite_id` identifies the complete accepted derivation
/// policy, not merely the signature algorithm.
pub struct Ed25519Verifier<'a> {
    suite_id: &'a str,
}

impl<'a> Ed25519Verifier<'a> {
    pub fn new(suite_id: &'a str) -> Self {
        Self { suite_id }
    }
}

impl SignatureVerifier for Ed25519Verifier<'_> {
    fn suite_id(&self) -> &str {
        self.suite_id
    }

    fn verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> std::result::Result<bool, AlgorithmError> {
        let Ok(public_key) = <&[u8; 32]>::try_from(public_key) else {
            return Ok(false);
        };
        let Ok(key) = VerifyingKey::from_bytes(public_key) else {
            return Ok(false);
        };
        let Ok(signature) = Signature::try_from(signature) else {
            return Ok(false);
        };

        Ok(key.verify_strict(message, &signature).is_ok())
    }
}

/// Authenticated commitment and replay state. Fields cannot be modified through
/// request data. Serialization is for trusted contract storage, not registration.
#[derive(BorshDeserialize, BorshSerialize)]
pub struct Enrollment {
    commitment: Commitment,
    next_nonce: u64,
    active: bool,
}

impl Enrollment {
    /// Register revision one for the immediate caller on this contract/network.
    /// The application supplies its accepted Ed25519 suite and must reject an
    /// already occupied owner slot rather than calling this again to reset it.
    pub fn register(commitment: Commitment, accepted_suite_id: &str) -> Result<Self> {
        Self::register_at(
            commitment,
            accepted_suite_id,
            &Runtime::current(),
            &env::predecessor_account_id(),
        )
    }

    /// Replace the root through owner authorization. The enrollment ID is stable;
    /// the revision must increase by exactly one. Replacement also reactivates a
    /// revoked enrollment and resets its nonce in the new signed revision domain.
    pub fn replace(&mut self, commitment: Commitment) -> Result<()> {
        self.replace_at(
            commitment,
            &Runtime::current(),
            &env::predecessor_account_id(),
        )
    }

    /// Disable signed actions without deleting the revision or replay history.
    pub fn revoke(&mut self) -> Result<()> {
        self.revoke_as(&env::predecessor_account_id())
    }

    /// Verify an authorization against the registered commitment and the expected
    /// operation/payload without changing state. Checks include active status,
    /// domain, revision, the current next nonce, expiration, membership and signature.
    ///
    /// This is safe to use in a NEAR view call. A successful result does not reserve
    /// the nonce or grant application permissions. For one-time acceptance, use
    /// [`Self::verify_and_consume`] in the receipt that applies the application change.
    pub fn verify(
        &self,
        operation: &str,
        payload: &[u8],
        authorization: &Authorization,
    ) -> Result<()> {
        self.verify_at(operation, payload, authorization, &Runtime::current())
            .map(|_| ())
    }

    /// Check the independently constructed operation/payload, signature and proof,
    /// then consume the next nonce. Expiration is Unix milliseconds, compared to
    /// NEAR block time. The surrounding method must abort on failure and persist
    /// this record together with its application change in the same receipt.
    ///
    /// Do not obtain `operation` or `payload` from the submitted authorization:
    /// construct them from the actual typed function arguments. Permissions and
    /// application bounds remain the responsibility of that function.
    pub fn verify_and_consume(
        &mut self,
        operation: &str,
        payload: &[u8],
        authorization: &Authorization,
    ) -> Result<()> {
        self.verify_and_consume_at(operation, payload, authorization, &Runtime::current())
    }

    /// Public data for preparing an offchain signing request. No tree, private key,
    /// feature coordinates or biometric image is retained in contract state.
    pub fn view(&self) -> EnrollmentView {
        EnrollmentView {
            commitment: self.commitment.clone(),
            active: self.active,
            next_nonce: self.next_nonce,
        }
    }

    fn register_at(
        commitment: Commitment,
        accepted_suite_id: &str,
        runtime: &Runtime,
        predecessor: &AccountId,
    ) -> Result<Self> {
        validate_registration(&commitment, accepted_suite_id, runtime, predecessor)?;
        if commitment.revision != 1 {
            return Err(Error::InvalidInput(
                "initial enrollment revision must be one",
            ));
        }

        Ok(Self {
            commitment,
            next_nonce: 0,
            active: true,
        })
    }

    fn replace_at(
        &mut self,
        commitment: Commitment,
        runtime: &Runtime,
        predecessor: &AccountId,
    ) -> Result<()> {
        require_owner(&self.commitment.owner_id, predecessor)?;
        validate_registration(&commitment, &self.commitment.suite_id, runtime, predecessor)?;
        let next_revision = self
            .commitment
            .revision
            .checked_add(1)
            .ok_or(Error::InvalidInput("enrollment revision is exhausted"))?;
        if commitment.owner_id != self.commitment.owner_id
            || commitment.enrollment_id != self.commitment.enrollment_id
            || commitment.revision != next_revision
        {
            return Err(Error::InvalidInput(
                "replacement must preserve owner and ID and advance revision by one",
            ));
        }

        self.commitment = commitment;
        self.next_nonce = 0;
        self.active = true;
        Ok(())
    }

    fn revoke_as(&mut self, predecessor: &AccountId) -> Result<()> {
        require_owner(&self.commitment.owner_id, predecessor)?;
        self.active = false;
        Ok(())
    }

    fn verify_and_consume_at(
        &mut self,
        operation: &str,
        payload: &[u8],
        authorization: &Authorization,
        runtime: &Runtime,
    ) -> Result<()> {
        self.next_nonce = self.verify_at(operation, payload, authorization, runtime)?;
        Ok(())
    }

    fn verify_at(
        &self,
        operation: &str,
        payload: &[u8],
        authorization: &Authorization,
        runtime: &Runtime,
    ) -> Result<u64> {
        runtime.validate_domain(&self.commitment)?;
        let next_nonce = self
            .next_nonce
            .checked_add(1)
            .ok_or(Error::InvalidInput("authorization nonce is exhausted"))?;
        let expected = Action {
            operation: operation.to_owned(),
            payload: payload.to_vec(),
            nonce: self.next_nonce,
            expires_at_ms: authorization.action.expires_at_ms,
        }
        .to_core()?;
        let trusted = self.commitment.to_core()?;
        let submitted = authorization.to_core()?;
        let status = if self.active {
            EnrollmentStatus::Active
        } else {
            EnrollmentStatus::Revoked
        };

        authorization::verify(
            &Ed25519Verifier::new(&self.commitment.suite_id),
            &trusted,
            status,
            &expected,
            &submitted,
            runtime.now_ms,
            &verification_limits(),
        )?;

        Ok(next_nonce)
    }
}

fn validate_registration(
    commitment: &Commitment,
    accepted_suite_id: &str,
    runtime: &Runtime,
    predecessor: &AccountId,
) -> Result<()> {
    commitment.validate()?;
    require_owner(&commitment.owner_id, predecessor)?;
    runtime.validate_domain(commitment)?;
    if commitment.suite_id != accepted_suite_id {
        return Err(Error::InvalidInput("unsupported cryptographic suite"));
    }
    Ok(())
}

fn require_owner(owner_id: &AccountId, predecessor: &AccountId) -> Result<()> {
    if predecessor != owner_id {
        return Err(Error::InvalidInput(
            "only the enrollment owner may change registration",
        ));
    }
    Ok(())
}

struct Runtime {
    network: String,
    contract_id: AccountId,
    now_ms: u64,
}

impl Runtime {
    fn current() -> Self {
        Self {
            network: format!("near:{}", env::chain_id()),
            contract_id: env::current_account_id(),
            now_ms: env::block_timestamp_ms(),
        }
    }

    fn validate_domain(&self, commitment: &Commitment) -> Result<()> {
        if commitment.network != self.network || commitment.contract_id != self.contract_id {
            return Err(Error::InvalidInput(
                "enrollment belongs to a different NEAR network or contract",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
