//! Runs core-produced authorizations through an actual local NEAR runtime.
//! Candidate bytes here are synthetic protocol fixtures, not biometric evidence.

use std::{
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
};

use anyhow::{Context, Result, ensure};
use gnorix::{
    core::{
        authorization,
        biometrics::{Candidate, Candidates},
        enrollment::{self, Domain, EnrollmentContext, EnrollmentSpec},
        package::EnrollmentPackage,
        policy::{Limits, SecretPolicy},
    },
    near::{Action, Authorization, Commitment, EnrollmentView, verification_limits},
};
use gnorix_example_support::example_crypto::{ExampleCrypto, SUITE_ID};
use near_workspaces::{
    Account, Contract, Worker, network::Sandbox, result::ExecutionFinalResult, types::NearToken,
};
use serde_json::json;

const PROFILE_ID: [u8; 32] = [19; 32];
const ENROLLMENT_ID: [u8; 32] = [27; 32];
const OPERATION: &str = "counter.increment";

struct Scenario {
    worker: Worker<Sandbox>,
    contract: Contract,
    owner: Account,
    relayer: Account,
    package: EnrollmentPackage,
}

impl Scenario {
    async fn start() -> Result<Self> {
        let wasm_path = std::env::var_os("GNORIX_NEAR_CONTRACT_WASM")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/near/gnorix_near_counter.wasm")
            });
        let wasm = std::fs::read(&wasm_path)
            .with_context(|| format!("build the NEAR contract first: {}", wasm_path.display()))?;
        let worker = near_workspaces::sandbox_with_version("2.14.1").await?;
        let contract = worker.dev_deploy(&wasm).await?;
        contract.call("new").transact().await?.into_result()?;
        let owner = worker.dev_create_account().await?;
        let relayer = worker.dev_create_account().await?;
        let network = format!("near:{}", worker.status().await?.chain_id);
        println!("Sandbox network: {network}");
        let package = enrollment_package(&network, contract.id().as_str(), owner.id().as_str(), 1)?;

        Ok(Self {
            worker,
            contract,
            owner,
            relayer,
            package,
        })
    }

    async fn enrollment(&self) -> Result<Option<EnrollmentView>> {
        self.contract
            .view("get_enrollment")
            .args_json(json!({"owner_id": self.owner.id()}))
            .await?
            .json()
            .map_err(Into::into)
    }

    async fn counter(&self) -> Result<u64> {
        let value: Option<String> = self
            .contract
            .view("get_counter")
            .args_json(json!({"owner_id": self.owner.id()}))
            .await?
            .json()?;
        Ok(value.context("owner should be registered")?.parse()?)
    }

    async fn expires_at(&self) -> Result<u64> {
        Ok(self.worker.view_block().await?.timestamp() / 1_000_000 + 300_000)
    }

    async fn register(
        &self,
        caller: &Account,
        commitment: &Commitment,
        deposit: NearToken,
    ) -> Result<ExecutionFinalResult> {
        Ok(caller
            .call(self.contract.id(), "register_enrollment")
            .args_json(json!({"commitment": commitment}))
            .deposit(deposit)
            .max_gas()
            .transact()
            .await?)
    }

    async fn submit(
        &self,
        delta: u64,
        authorization: &Authorization,
    ) -> Result<ExecutionFinalResult> {
        Ok(self
            .relayer
            .call(self.contract.id(), "increment")
            .args_json(json!({
                "owner_id": self.owner.id(),
                "delta": delta.to_string(),
                "authorization": authorization,
            }))
            .max_gas()
            .transact()
            .await?)
    }

    async fn verify(
        &self,
        operation: &str,
        payload: &[u8],
        authorization: &Authorization,
    ) -> Result<bool> {
        Ok(self
            .contract
            .view("verify_authorization")
            .args_json(json!({
                "owner_id": self.owner.id(),
                "operation": operation,
                "payload": payload,
                "authorization": authorization,
            }))
            .await?
            .json()?)
    }
}

fn candidates(tags: &[u8]) -> Candidates<'_> {
    Box::new(
        tags.iter()
            .map(|tag| Ok(Candidate::new(vec![*tag; 32]).unwrap())),
    )
}

fn limits() -> Limits {
    Limits {
        candidate_attempts: NonZeroUsize::new(16).unwrap(),
        ..verification_limits()
    }
}

fn enrollment_package(
    network: &str,
    contract: &str,
    owner: &str,
    revision: u64,
) -> Result<EnrollmentPackage> {
    let context = EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain {
                network: network.to_owned(),
                verifier: contract.to_owned(),
            },
            subject: owner.to_owned(),
            id: ENROLLMENT_ID,
            revision: NonZeroU64::new(revision).unwrap(),
            public_salt: vec![31; 32],
            secret_policy: SecretPolicy::Absent,
        },
        profile_id: PROFILE_ID,
        suite_id: SUITE_ID.to_owned(),
    };
    let enrolled = enrollment::enroll_candidates(
        &ExampleCrypto,
        candidates(&[1, 2, 3]),
        context,
        None,
        &limits(),
    )?;
    Ok(EnrollmentPackage::from_enrollment(&enrolled, &limits())?)
}

fn sign(
    package: &EnrollmentPackage,
    delta: u64,
    nonce: u64,
    expires_at_ms: u64,
) -> Result<Authorization> {
    sign_operation(package, OPERATION, delta, nonce, expires_at_ms)
}

fn sign_operation(
    package: &EnrollmentPackage,
    operation: &str,
    delta: u64,
    nonce: u64,
    expires_at_ms: u64,
) -> Result<Authorization> {
    sign_action(
        package,
        Action {
            operation: operation.to_owned(),
            payload: delta.to_be_bytes().to_vec(),
            nonce,
            expires_at_ms,
        },
    )
}

fn sign_action(package: &EnrollmentPackage, action: Action) -> Result<Authorization> {
    // The first candidate misses, the second matches; the signer must stop there.
    let stream = candidates(&[99, 2]).chain(std::iter::once_with(|| {
        panic!("signer consumed a candidate after finding the enrolled key")
    }));
    let signed = authorization::sign_candidates(
        &ExampleCrypto,
        Box::new(stream),
        PROFILE_ID,
        package,
        action.to_core()?,
        None,
        &limits(),
    )?;
    Ok(Authorization::from_core(&signed)?)
}

fn rejected(result: ExecutionFinalResult, reason: &str) -> Result<()> {
    ensure!(
        result.is_failure(),
        "{reason}: transaction unexpectedly succeeded"
    );
    println!("Rejected {reason}: {}", result.into_result().unwrap_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires the built example Wasm and a local NEAR sandbox"]
async fn core_authorizations_execute_and_fail_closed_in_near_wasm() -> Result<()> {
    let mut scenario = Scenario::start().await?;
    let commitment = Commitment::from_core(scenario.package.commitment())?;
    let deposit = NearToken::from_millinear(100);

    rejected(
        scenario
            .register(&scenario.relayer, &commitment, deposit)
            .await?,
        "unauthorized registration",
    )?;
    let mut wrong_network = commitment.clone();
    wrong_network.network = "near:wrong-network".into();
    rejected(
        scenario
            .register(&scenario.owner, &wrong_network, deposit)
            .await?,
        "wrong network",
    )?;
    let mut wrong_contract = commitment.clone();
    wrong_contract.contract_id = "other-contract.testnet".parse()?;
    rejected(
        scenario
            .register(&scenario.owner, &wrong_contract, deposit)
            .await?,
        "wrong contract",
    )?;
    rejected(
        scenario
            .register(&scenario.owner, &commitment, NearToken::from_yoctonear(0))
            .await?,
        "unpaid storage",
    )?;
    ensure!(
        scenario.enrollment().await?.is_none(),
        "failed registration retained state"
    );

    scenario
        .register(&scenario.owner, &commitment, deposit)
        .await?
        .into_result()?;
    rejected(
        scenario
            .register(&scenario.owner, &commitment, deposit)
            .await?,
        "duplicate registration",
    )?;
    let expires = scenario.expires_at().await?;
    let current = scenario.enrollment().await?.unwrap();
    ensure!(current.active, "registered enrollment is revoked");
    ensure!(
        current.commitment.to_core()? == *scenario.package.commitment(),
        "retained package differs from the registered commitment"
    );
    let approved_action = Action {
        operation: OPERATION.to_owned(),
        payload: 7_u64.to_be_bytes().to_vec(),
        nonce: current.next_nonce,
        expires_at_ms: expires,
    };
    let valid = sign_action(&scenario.package, approved_action)?;
    for _ in 0..2 {
        ensure!(
            scenario
                .verify(OPERATION, &7_u64.to_be_bytes(), &valid)
                .await?
        );
    }
    ensure!(
        !scenario
            .verify(OPERATION, &8_u64.to_be_bytes(), &valid)
            .await?
    );
    ensure!(scenario.counter().await? == 0);
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == current.next_nonce);

    let document = b"Terms version 2\0\xff";
    let arbitrary = sign_action(
        &scenario.package,
        Action {
            operation: "document.approve".into(),
            payload: document.to_vec(),
            nonce: current.next_nonce,
            expires_at_ms: expires,
        },
    )?;
    ensure!(
        scenario
            .verify("document.approve", document, &arbitrary)
            .await?,
        "verification must support arbitrary application data"
    );

    let mut larger_replacement = commitment.clone();
    larger_replacement.revision = 2;
    larger_replacement.public_salt = vec![43; 64];
    rejected(
        scenario
            .owner
            .call(scenario.contract.id(), "replace_enrollment")
            .args_json(json!({"commitment": larger_replacement}))
            .max_gas()
            .transact()
            .await?,
        "unpaid replacement storage",
    )?;
    ensure!(
        scenario.enrollment().await?.unwrap().commitment == commitment,
        "failed replacement retained state"
    );

    rejected(
        scenario.submit(8, &valid).await?,
        "tampered application payload",
    )?;
    let mut bad_signature = valid.clone();
    bad_signature.signature[0] ^= 1;
    rejected(
        scenario.submit(7, &bad_signature).await?,
        "invalid signature",
    )?;
    let mut bad_path = valid.clone();
    bad_path.siblings[0][0] ^= 1;
    rejected(scenario.submit(7, &bad_path).await?, "invalid Merkle path")?;
    rejected(
        scenario
            .submit(7, &sign(&scenario.package, 7, 0, 1)?)
            .await?,
        "expired authorization",
    )?;
    let disallowed = sign(&scenario.package, 101, 0, expires)?;
    ensure!(
        scenario
            .verify(OPERATION, &101_u64.to_be_bytes(), &disallowed)
            .await?,
        "signature verification must not impose counter permissions"
    );
    rejected(
        scenario.submit(101, &disallowed).await?,
        "application permission",
    )?;
    ensure!(
        scenario.enrollment().await?.unwrap().next_nonce == current.next_nonce,
        "application failure must roll back nonce consumption"
    );
    rejected(
        scenario
            .submit(
                7,
                &sign_operation(&scenario.package, "counter.reset", 7, 0, expires)?,
            )
            .await?,
        "wrong operation",
    )?;
    rejected(
        scenario
            .submit(7, &sign(&scenario.package, 7, 1, expires)?)
            .await?,
        "future nonce",
    )?;

    let other_domain = enrollment_package(
        "near:another-network",
        scenario.contract.id().as_str(),
        scenario.owner.id().as_str(),
        1,
    )?;
    rejected(
        scenario
            .submit(7, &sign(&other_domain, 7, 0, expires)?)
            .await?,
        "cross-network signature",
    )?;
    ensure!(scenario.counter().await? == 0);
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 0);

    let success = scenario.submit(7, &valid).await?;
    println!("Successful authorization gas: {}", success.total_gas_burnt);
    let counter: String = success.into_result()?.json()?;
    ensure!(counter == "7");
    ensure!(
        !scenario
            .verify(OPERATION, &7_u64.to_be_bytes(), &valid)
            .await?,
        "a consumed authorization must fail the view check too"
    );
    rejected(scenario.submit(7, &valid).await?, "replay")?;
    ensure!(scenario.enrollment().await?.unwrap().next_nonce == 1);

    let replacement = enrollment_package(
        &commitment.network,
        scenario.contract.id().as_str(),
        scenario.owner.id().as_str(),
        2,
    )?;
    let replacement_commitment = Commitment::from_core(replacement.commitment())?;
    rejected(
        scenario
            .relayer
            .call(scenario.contract.id(), "replace_enrollment")
            .args_json(json!({"commitment": replacement_commitment}))
            .deposit(deposit)
            .max_gas()
            .transact()
            .await?,
        "unauthorized replacement",
    )?;
    scenario
        .owner
        .call(scenario.contract.id(), "replace_enrollment")
        .args_json(json!({"commitment": replacement_commitment}))
        .deposit(deposit)
        .max_gas()
        .transact()
        .await?
        .into_result()?;
    rejected(
        scenario
            .submit(7, &sign(&scenario.package, 7, 1, expires)?)
            .await?,
        "superseded revision",
    )?;
    scenario.package = replacement;
    let nonce = scenario.enrollment().await?.unwrap().next_nonce;
    scenario
        .submit(9, &sign(&scenario.package, 9, nonce, expires)?)
        .await?
        .into_result()?;
    ensure!(scenario.counter().await? == 16);

    scenario
        .owner
        .call(scenario.contract.id(), "revoke_enrollment")
        .args_json(json!({}))
        .transact()
        .await?
        .into_result()?;
    let revoked = scenario.enrollment().await?.unwrap();
    ensure!(!revoked.active);
    rejected(
        scenario
            .submit(5, &sign(&scenario.package, 5, revoked.next_nonce, expires)?)
            .await?,
        "revoked enrollment",
    )?;
    ensure!(scenario.counter().await? == 16);

    Ok(())
}
