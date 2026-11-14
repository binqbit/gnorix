//! Core authorizations controlling an account that owns ordinary NEP-141 tokens.
//! Synthetic candidates isolate authorization and asynchronous receipt semantics.

use std::{
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
};

use anyhow::{Context, Result, ensure};
use gnorix::{
    core::{
        authorization,
        biometrics::Candidate,
        enrollment::{Domain, EnrollmentContext, EnrollmentSpec, enroll_candidates},
        package::EnrollmentPackage,
        policy::SecretPolicy,
    },
    near::{Action, Authorization, Commitment, EnrollmentView, verification_limits},
};
use gnorix_example_support::example_crypto::{ExampleCrypto, SUITE_ID};
use gnorix_near_wallet::{TRANSFER_OPERATION, Transfer};
use near_workspaces::{
    Account, Contract, Worker,
    network::Sandbox,
    operations::Function,
    result::ExecutionFinalResult,
    types::{Gas, NearToken},
};
use serde_json::{Value, json};

const SUPPLY: u128 = 10_000;
const PROFILE_ID: [u8; 32] = [51; 32];

struct Scenario {
    worker: Worker<Sandbox>,
    wallet: Contract,
    token: Contract,
    owner: Account,
    relayer: Account,
    recipient: Account,
    unregistered: Account,
    package: EnrollmentPackage,
}

impl Scenario {
    async fn start() -> Result<Self> {
        let worker = near_workspaces::sandbox_with_version("2.14.1").await?;
        let wallet = worker.dev_deploy(&wasm("gnorix_near_wallet.wasm")?).await?;
        let token = worker.dev_deploy(&wasm("gnorix_near_token.wasm")?).await?;
        let owner = worker.dev_create_account().await?;
        let relayer = worker.dev_create_account().await?;
        let recipient = worker.dev_create_account().await?;
        let unregistered = worker.dev_create_account().await?;
        rejected(
            owner
                .call(wallet.id(), "new")
                .args_json(json!({"owner_id": owner.id()}))
                .transact()
                .await?,
            "foreign wallet initialization",
        )?;
        wallet
            .call("new")
            .args_json(json!({"owner_id": owner.id()}))
            .transact()
            .await?
            .into_result()?;
        token
            .call("new")
            .args_json(json!({"owner_id": wallet.id(), "total_supply": SUPPLY.to_string()}))
            .transact()
            .await?
            .into_result()?;
        let network = format!("near:{}", worker.status().await?.chain_id);
        let package = package(&network, wallet.id().as_str(), owner.id().as_str(), 1)?;
        let scenario = Self {
            worker,
            wallet,
            token,
            owner,
            relayer,
            recipient,
            unregistered,
            package,
        };
        scenario
            .register_storage(scenario.recipient.id().as_str())
            .await?;
        Ok(scenario)
    }

    async fn enrollment(&self) -> Result<Option<EnrollmentView>> {
        Ok(self.wallet.view("get_enrollment").await?.json()?)
    }

    async fn status(&self) -> Result<Value> {
        Ok(self.wallet.view("get_transfer_status").await?.json()?)
    }

    async fn balance(&self, account: &str) -> Result<u128> {
        let value: String = self
            .token
            .view("ft_balance_of")
            .args_json(json!({"account_id": account}))
            .await?
            .json()?;
        Ok(value.parse()?)
    }

    async fn expires(&self) -> Result<u64> {
        Ok(self.worker.view_block().await?.timestamp() / 1_000_000 + 600_000)
    }

    async fn register_storage(&self, account: &str) -> Result<()> {
        let result = self
            .owner
            .call(self.token.id(), "storage_deposit")
            .args_json(json!({"account_id": account, "registration_only": true}))
            .deposit(NearToken::from_millinear(100))
            .max_gas()
            .transact()
            .await?;
        successful(&result, "recipient storage registration")
    }

    fn transfer(&self, amount: u128) -> Transfer {
        Transfer {
            token_id: self.token.id().as_str().parse().unwrap(),
            receiver_id: self.recipient.id().as_str().parse().unwrap(),
            amount: amount.into(),
            memo: Some("Gnorix wallet transfer".into()),
        }
    }

    async fn submit(
        &self,
        transfer: &Transfer,
        authorization: &Authorization,
    ) -> Result<ExecutionFinalResult> {
        Ok(self
            .relayer
            .call(self.wallet.id(), "transfer")
            .args_json(json!({"transfer": transfer, "authorization": authorization}))
            .max_gas()
            .transact()
            .await?)
    }

    async fn verify(&self, transfer: &Transfer, authorization: &Authorization) -> Result<bool> {
        Ok(self
            .wallet
            .view("verify_transfer")
            .args_json(json!({"transfer": transfer, "authorization": authorization}))
            .await?
            .json()?)
    }
}

fn wasm(name: &str) -> Result<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/near")
        .join(name);
    std::fs::read(&path)
        .with_context(|| format!("build the NEAR example first: {}", path.display()))
}

fn package(network: &str, contract: &str, owner: &str, revision: u64) -> Result<EnrollmentPackage> {
    let context = EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain {
                network: network.into(),
                verifier: contract.into(),
            },
            subject: owner.into(),
            id: [71; 32],
            revision: NonZeroU64::new(revision).unwrap(),
            public_salt: vec![72; 32],
            secret_policy: SecretPolicy::Absent,
        },
        profile_id: PROFILE_ID,
        suite_id: SUITE_ID.into(),
    };
    let mut limits = verification_limits();
    limits.candidate_attempts = NonZeroUsize::new(3).unwrap();
    let candidates = Box::new((1..=3).map(|value| Ok(Candidate::new(vec![value; 32]).unwrap())));
    let enrollment = enroll_candidates(&ExampleCrypto, candidates, context, None, &limits)?;
    Ok(EnrollmentPackage::from_enrollment(&enrollment, &limits)?)
}

fn signed(
    package: &EnrollmentPackage,
    transfer: &Transfer,
    nonce: u64,
    expires_at_ms: u64,
) -> Result<Authorization> {
    signed_action(
        package,
        Action {
            operation: TRANSFER_OPERATION.into(),
            payload: transfer.signing_payload(),
            nonce,
            expires_at_ms,
        },
    )
}

fn signed_action(package: &EnrollmentPackage, action: Action) -> Result<Authorization> {
    // The first candidate misses, the second matches. Do not evaluate any
    // further candidate once its enrolled key has been reconstructed.
    let candidates = [99, 2]
        .into_iter()
        .map(|value| Ok(Candidate::new(vec![value; 32]).unwrap()))
        .chain(std::iter::once_with(|| {
            panic!("signer consumed a candidate after finding the enrolled key")
        }));
    let mut limits = verification_limits();
    limits.candidate_attempts = NonZeroUsize::new(3).unwrap();
    let authorization = authorization::sign_candidates(
        &ExampleCrypto,
        Box::new(candidates),
        PROFILE_ID,
        package,
        action.to_core()?,
        None,
        &limits,
    )?;
    Ok(Authorization::from_core(&authorization)?)
}

fn rejected(result: ExecutionFinalResult, context: &str) -> Result<()> {
    ensure!(result.is_failure(), "{context}: unexpectedly succeeded");
    Ok(())
}

fn successful(result: &ExecutionFinalResult, context: &str) -> Result<()> {
    ensure!(
        result.is_success() && result.receipt_failures().is_empty(),
        "{context}: {result:?}"
    );
    Ok(())
}

fn expected_status(revision: u64, nonce: u64, status: &str) -> Value {
    json!({"revision": revision.to_string(), "nonce": nonce.to_string(), "status": status})
}

async fn owner_registration(scenario: &Scenario) -> Result<()> {
    let owner: String = scenario.wallet.view("get_owner").await?.json()?;
    ensure!(owner == scenario.owner.id().as_str());
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == SUPPLY);
    ensure!(scenario.balance(scenario.owner.id().as_str()).await? == 0);
    ensure!(scenario.enrollment().await?.is_none());
    let commitment = Commitment::from_core(scenario.package.commitment())?;
    rejected(
        scenario
            .relayer
            .call(scenario.wallet.id(), "register_enrollment")
            .args_json(json!({"commitment": commitment}))
            .deposit(NearToken::from_millinear(100))
            .transact()
            .await?,
        "foreign registration",
    )?;
    let mut wrong_network = commitment.clone();
    wrong_network.network = "near:wrong-network".into();
    rejected(
        scenario
            .owner
            .call(scenario.wallet.id(), "register_enrollment")
            .args_json(json!({"commitment": wrong_network}))
            .deposit(NearToken::from_millinear(100))
            .transact()
            .await?,
        "wrong network registration",
    )?;
    let mut wrong_domain = commitment.clone();
    wrong_domain.contract_id = scenario.token.id().as_str().parse()?;
    rejected(
        scenario
            .owner
            .call(scenario.wallet.id(), "register_enrollment")
            .args_json(json!({"commitment": wrong_domain}))
            .deposit(NearToken::from_millinear(100))
            .transact()
            .await?,
        "wrong contract registration",
    )?;
    rejected(
        scenario
            .owner
            .call(scenario.wallet.id(), "register_enrollment")
            .args_json(json!({"commitment": commitment}))
            .transact()
            .await?,
        "unpaid enrollment storage",
    )?;
    ensure!(scenario.enrollment().await?.is_none());
    scenario
        .owner
        .call(scenario.wallet.id(), "register_enrollment")
        .args_json(json!({"commitment": commitment}))
        .deposit(NearToken::from_millinear(100))
        .transact()
        .await?
        .into_result()?;
    rejected(
        scenario
            .owner
            .call(scenario.wallet.id(), "register_enrollment")
            .args_json(json!({"commitment": commitment}))
            .deposit(NearToken::from_millinear(100))
            .transact()
            .await?,
        "duplicate registration",
    )?;
    rejected(
        scenario
            .relayer
            .call(scenario.wallet.id(), "revoke_enrollment")
            .transact()
            .await?,
        "foreign revocation",
    )?;
    let state = scenario
        .enrollment()
        .await?
        .context("owner enrollment missing")?;
    ensure!(state.commitment == commitment && state.active && state.next_nonce == 0);
    Ok(())
}

async fn replacement_requires_storage_payment(scenario: &Scenario) -> Result<()> {
    let original = scenario
        .enrollment()
        .await?
        .context("owner enrollment missing")?;
    let status = scenario.status().await?;
    let mut larger = original.commitment.clone();
    larger.revision += 1;
    larger.public_salt = vec![43; 64];

    let unpaid = scenario
        .owner
        .call(scenario.wallet.id(), "replace_enrollment")
        .args_json(json!({"commitment": larger}))
        .max_gas()
        .transact()
        .await?;
    ensure!(
        format!("{unpaid:?}").contains("insufficient storage deposit"),
        "expected replacement storage payment rejection",
    );
    rejected(unpaid, "unpaid replacement storage")?;
    ensure!(scenario.enrollment().await? == Some(original));
    ensure!(scenario.status().await? == status);
    Ok(())
}

async fn rejection_preserves_state(
    scenario: &Scenario,
    transfer: &Transfer,
    valid: &Authorization,
    expires: u64,
) -> Result<()> {
    for _ in 0..2 {
        ensure!(scenario.verify(transfer, valid).await?);
    }
    let mutate: [fn(&mut Transfer); 4] = [
        |value| value.token_id = "different-token.testnet".parse().unwrap(),
        |value| value.receiver_id = "different-recipient.testnet".parse().unwrap(),
        |value| value.amount.0 += 1,
        |value| value.memo = None,
    ];
    for mutation in mutate {
        let mut changed = transfer.clone();
        mutation(&mut changed);
        ensure!(!scenario.verify(&changed, valid).await?);
        rejected(
            scenario.submit(&changed, valid).await?,
            "altered signed transfer",
        )?;
    }
    let mut forged = valid.clone();
    forged.signature[0] ^= 1;
    rejected(
        scenario.submit(transfer, &forged).await?,
        "invalid signature",
    )?;
    let mut bad_path = valid.clone();
    bad_path.siblings[0][0] ^= 1;
    ensure!(!scenario.verify(transfer, &bad_path).await?);
    rejected(
        scenario.submit(transfer, &bad_path).await?,
        "invalid Merkle path",
    )?;
    rejected(
        scenario
            .submit(transfer, &signed(&scenario.package, transfer, 0, 1)?)
            .await?,
        "expired signature",
    )?;
    let context = &scenario.package.commitment().context;
    let foreign = package(
        &context.spec.domain.network,
        scenario.token.id().as_str(),
        scenario.owner.id().as_str(),
        1,
    )?;
    rejected(
        scenario
            .submit(transfer, &signed(&foreign, transfer, 0, expires)?)
            .await?,
        "cross-contract signature",
    )?;
    let foreign_network = package(
        "near:another-network",
        scenario.wallet.id().as_str(),
        scenario.owner.id().as_str(),
        1,
    )?;
    rejected(
        scenario
            .submit(transfer, &signed(&foreign_network, transfer, 0, expires)?)
            .await?,
        "cross-network signature",
    )?;
    let wrong_operation = signed_action(
        &scenario.package,
        Action {
            operation: "wallet.other_operation".into(),
            payload: transfer.signing_payload(),
            nonce: 0,
            expires_at_ms: expires,
        },
    )?;
    ensure!(!scenario.verify(transfer, &wrong_operation).await?);
    rejected(
        scenario.submit(transfer, &wrong_operation).await?,
        "signature approving a different operation",
    )?;
    let future_nonce = signed(&scenario.package, transfer, 1, expires)?;
    ensure!(!scenario.verify(transfer, &future_nonce).await?);
    rejected(
        scenario.submit(transfer, &future_nonce).await?,
        "signature approving a future nonce",
    )?;
    let mut empty = transfer.clone();
    empty.amount = 0.into();
    let signed_empty = signed(&scenario.package, &empty, 0, expires)?;
    ensure!(
        scenario.verify(&empty, &signed_empty).await?,
        "read-only signature verification must not enforce transfer business rules",
    );
    rejected(scenario.submit(&empty, &signed_empty).await?, "zero amount")?;
    rejected(
        scenario
            .relayer
            .call(scenario.wallet.id(), "on_transfer")
            .args_json(json!({"revision": "1", "nonce": "0"}))
            .transact()
            .await?,
        "forged private callback",
    )?;

    let low_gas = scenario
        .relayer
        .call(scenario.wallet.id(), "transfer")
        .args_json(json!({"transfer": transfer, "authorization": valid}))
        .gas(Gas::from_tgas(30))
        .transact()
        .await?;
    ensure!(
        format!("{low_gas:?}").contains("insufficient gas for transfer and callback"),
        "expected explicit scheduling gas rejection"
    );
    rejected(low_gas, "insufficient scheduling gas")?;
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 0);
    ensure!(scenario.status().await? == expected_status(0, 0, "idle"));
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == SUPPLY);
    ensure!(scenario.balance(scenario.recipient.id().as_str()).await? == 0);
    Ok(())
}

async fn pending_requests_reject_overlap(
    scenario: &Scenario,
    transfer: &Transfer,
    valid: &Authorization,
    expires: u64,
) -> Result<()> {
    let next = signed(&scenario.package, transfer, 1, expires)?;
    let first = || {
        Function::new("transfer")
            .args_json(json!({"transfer": transfer, "authorization": valid}))
            .gas(Gas::from_tgas(80))
    };
    let overlapped = scenario
        .owner
        .batch(scenario.wallet.id())
        .call(first())
        .call(
            Function::new("transfer")
                .args_json(json!({"transfer": transfer, "authorization": next}))
                .gas(Gas::from_tgas(80)),
        )
        .transact()
        .await?;
    ensure!(format!("{overlapped:?}").contains("a transfer is still pending"));
    rejected(overlapped, "overlapping transfers in one receipt")?;
    let context = &scenario.package.commitment().context;
    let replacement = package(
        &context.spec.domain.network,
        scenario.wallet.id().as_str(),
        scenario.owner.id().as_str(),
        2,
    )?;
    let rotation = scenario
        .owner
        .batch(scenario.wallet.id())
        .call(first())
        .call(
            Function::new("replace_enrollment")
                .args_json(json!({"commitment": Commitment::from_core(replacement.commitment())?}))
                .deposit(NearToken::from_millinear(100))
                .gas(Gas::from_tgas(40)),
        )
        .transact()
        .await?;
    ensure!(format!("{rotation:?}").contains("a transfer is still pending"));
    rejected(rotation, "rotation during a pending transfer")?;
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 0);
    ensure!(scenario.status().await? == expected_status(0, 0, "idle"));
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == SUPPLY);
    Ok(())
}

async fn relayed_transfer(
    scenario: &Scenario,
    transfer: &Transfer,
    valid: &Authorization,
) -> Result<()> {
    let result = scenario.submit(transfer, valid).await?;
    successful(&result, "relayed wallet transfer")?;
    let events: Vec<Value> = result
        .logs()
        .into_iter()
        .filter_map(|line| line.strip_prefix("EVENT_JSON:"))
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        events
            == vec![
                json!({"standard":"nep141","version":"1.0.0","event":"ft_transfer","data":[{
                    "old_owner_id": scenario.wallet.id(), "new_owner_id": scenario.recipient.id(), "amount": transfer.amount.0.to_string(), "memo": transfer.memo,
                }]})
            ]
    );
    ensure!(result.json::<Value>()? == expected_status(1, 0, "succeeded"));
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == SUPPLY - transfer.amount.0);
    ensure!(scenario.balance(scenario.recipient.id().as_str()).await? == transfer.amount.0);
    ensure!(scenario.balance(scenario.owner.id().as_str()).await? == 0);
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 1);
    ensure!(!scenario.verify(transfer, valid).await?);
    rejected(
        scenario.submit(transfer, valid).await?,
        "successful transfer replay",
    )?;
    Ok(())
}

async fn external_failure_consumes_nonce(scenario: &Scenario, expires: u64) -> Result<()> {
    let mut transfer = scenario.transfer(25);
    transfer.receiver_id = scenario.unregistered.id().as_str().parse()?;
    let signed_failure = signed(&scenario.package, &transfer, 1, expires)?;
    let before = scenario.balance(scenario.wallet.id().as_str()).await?;
    let result = scenario.submit(&transfer, &signed_failure).await?;
    ensure!(
        result.is_success(),
        "wallet callback must report the external failure"
    );
    ensure!(
        !result.receipt_failures().is_empty(),
        "the unregistered recipient must reject the token receipt"
    );
    ensure!(result.json::<Value>()? == expected_status(1, 1, "failed"));
    ensure!(scenario.status().await? == expected_status(1, 1, "failed"));
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == before);
    ensure!(
        scenario
            .balance(scenario.unregistered.id().as_str())
            .await?
            == 0
    );
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 2);
    rejected(
        scenario.submit(&transfer, &signed_failure).await?,
        "failed transfer replay",
    )?;
    scenario
        .register_storage(scenario.unregistered.id().as_str())
        .await?;
    rejected(
        scenario.submit(&transfer, &signed_failure).await?,
        "repaired receiver does not restore consumed nonce",
    )?;
    let retry = signed(&scenario.package, &transfer, 2, expires)?;
    let result = scenario.submit(&transfer, &retry).await?;
    successful(&result, "fresh authorization after external failure")?;
    ensure!(result.json::<Value>()? == expected_status(1, 2, "succeeded"));
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 3);
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == before - 25);
    ensure!(
        scenario
            .balance(scenario.unregistered.id().as_str())
            .await?
            == 25
    );
    Ok(())
}

async fn rotate_and_revoke(scenario: &Scenario, expires: u64) -> Result<()> {
    let context = &scenario.package.commitment().context;
    let replacement = package(
        &context.spec.domain.network,
        scenario.wallet.id().as_str(),
        scenario.owner.id().as_str(),
        2,
    )?;
    let commitment = Commitment::from_core(replacement.commitment())?;
    rejected(
        scenario
            .relayer
            .call(scenario.wallet.id(), "replace_enrollment")
            .args_json(json!({"commitment": commitment}))
            .deposit(NearToken::from_millinear(100))
            .transact()
            .await?,
        "foreign rotation",
    )?;
    scenario
        .owner
        .call(scenario.wallet.id(), "replace_enrollment")
        .args_json(json!({"commitment": commitment}))
        .deposit(NearToken::from_millinear(100))
        .transact()
        .await?
        .into_result()?;
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 0);
    ensure!(
        scenario.status().await? == expected_status(1, 2, "succeeded"),
        "rotation must preserve transfer history"
    );
    let transfer = scenario.transfer(10);
    rejected(
        scenario
            .submit(
                &transfer,
                &signed(&scenario.package, &transfer, 3, expires)?,
            )
            .await?,
        "superseded enrollment",
    )?;
    let authorization = signed(&replacement, &transfer, 0, expires)?;
    let before = scenario.balance(scenario.wallet.id().as_str()).await?;
    // Both actions share one receipt: revocation happens after acceptance but
    // before the dispatched token receipt. It must not undo an accepted transfer.
    let revoked = scenario
        .owner
        .batch(scenario.wallet.id())
        .call(
            Function::new("transfer")
                .args_json(json!({"transfer": transfer, "authorization": authorization}))
                .gas(Gas::from_tgas(80)),
        )
        .call(Function::new("revoke_enrollment").gas(Gas::from_tgas(20)))
        .transact()
        .await?;
    successful(&revoked, "revoke after accepting a transfer")?;
    let state = scenario.enrollment().await?.unwrap();
    ensure!(!state.active && state.next_nonce == 1 && state.commitment == commitment);
    ensure!(scenario.status().await? == expected_status(2, 0, "succeeded"));
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == before - 10);
    let after_revoke = signed(&replacement, &transfer, 1, expires)?;
    ensure!(!scenario.verify(&transfer, &after_revoke).await?);
    rejected(
        scenario.submit(&transfer, &after_revoke).await?,
        "revoked enrollment",
    )?;
    ensure!(scenario.status().await? == expected_status(2, 0, "succeeded"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires the token/wallet Wasm builds and a local NEAR sandbox"]
async fn biometric_wallet_controls_standard_tokens_and_handles_async_failures() -> Result<()> {
    let scenario = Scenario::start().await?;
    owner_registration(&scenario).await?;
    replacement_requires_storage_payment(&scenario).await?;
    let expires = scenario.expires().await?;
    let transfer = scenario.transfer(100);
    let valid = signed(&scenario.package, &transfer, 0, expires)?;
    rejection_preserves_state(&scenario, &transfer, &valid, expires).await?;
    pending_requests_reject_overlap(&scenario, &transfer, &valid, expires).await?;
    relayed_transfer(&scenario, &transfer, &valid).await?;
    external_failure_consumes_nonce(&scenario, expires).await?;
    rotate_and_revoke(&scenario, expires).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires the token/wallet Wasm builds and a local NEAR sandbox"]
async fn native_token_unregistration_does_not_reset_wallet_authorization() -> Result<()> {
    let scenario = Scenario::start().await?;
    owner_registration(&scenario).await?;
    let expires = scenario.expires().await?;
    let transfer = scenario.transfer(100);
    let authorization = signed(&scenario.package, &transfer, 0, expires)?;
    relayed_transfer(&scenario, &transfer, &authorization).await?;
    let enrollment = scenario.enrollment().await?.unwrap();
    let status = scenario.status().await?;
    ensure!(enrollment.active && enrollment.next_nonce == 1);

    // The fixture retains the wallet's native account key. This ordinary NEP-145
    // call is authorized by that key, independently of Gnorix's enrolled root.
    let unregister = scenario
        .wallet
        .as_account()
        .call(scenario.token.id(), "storage_unregister")
        .args_json(json!({"force": true}))
        .deposit(NearToken::from_yoctonear(1))
        .max_gas()
        .transact()
        .await?;
    successful(&unregister, "native wallet-account forced token unregister")?;
    ensure!(unregister.json::<bool>()?);
    let storage: Value = scenario
        .token
        .view("storage_balance_of")
        .args_json(json!({"account_id": scenario.wallet.id()}))
        .await?
        .json()?;
    ensure!(storage.is_null());
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == 0);
    let supply: String = scenario.token.view("ft_total_supply").await?.json()?;
    ensure!(
        supply == "100",
        "forced unregister must burn the wallet's remaining tokens"
    );
    ensure!(scenario.enrollment().await? == Some(enrollment.clone()));
    ensure!(scenario.status().await? == status);

    scenario
        .register_storage(scenario.wallet.id().as_str())
        .await?;
    ensure!(scenario.enrollment().await? == Some(enrollment.clone()));
    ensure!(!scenario.verify(&transfer, &authorization).await?);
    rejected(
        scenario.submit(&transfer, &authorization).await?,
        "re-registering token storage must not reset a consumed nonce",
    )?;

    let funding = scenario
        .recipient
        .call(scenario.token.id(), "ft_transfer")
        .args_json(json!({"receiver_id": scenario.wallet.id(), "amount": "25"}))
        .deposit(NearToken::from_yoctonear(1))
        .max_gas()
        .transact()
        .await?;
    successful(&funding, "ordinary funding after wallet token registration")?;
    let fresh_transfer = scenario.transfer(5);
    let fresh = signed(
        &scenario.package,
        &fresh_transfer,
        enrollment.next_nonce,
        expires,
    )?;
    let completed = scenario.submit(&fresh_transfer, &fresh).await?;
    successful(&completed, "fresh nonce under the original enrollment")?;
    ensure!(completed.json::<Value>()? == expected_status(1, 1, "succeeded"));
    let after = scenario.enrollment().await?.unwrap();
    ensure!(after.commitment == enrollment.commitment && after.active && after.next_nonce == 2);
    ensure!(scenario.balance(scenario.wallet.id().as_str()).await? == 20);
    ensure!(scenario.balance(scenario.recipient.id().as_str()).await? == 80);
    Ok(())
}
