//! Real Sandbox execution and controlled transport-failure regression tests.

use std::{
    num::{NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
};

use gnorix::{
    core::{
        authorization,
        biometrics::Candidate,
        enrollment::{Domain, EnrollmentContext, EnrollmentSpec, enroll_candidates},
        package::EnrollmentPackage,
        policy::SecretPolicy,
    },
    near::{Action, verification_limits},
};
use gnorix_example_support::example_crypto::{ExampleCrypto, SUITE_ID};
use gnorix_near_wallet::TRANSFER_OPERATION;

use super::*;

// Synthetic candidates isolate chain orchestration. The demo itself obtains
// candidates from fingerprint images through the normal core pipeline.
fn enrollment(context: &WalletContext, value: u8) -> Result<EnrollmentPackage> {
    let context = EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain {
                network: format!("near:{}", context.chain_id),
                verifier: context.wallet_id.clone(),
            },
            subject: context.wallet_id.clone(),
            id: [value; 32],
            revision: NonZeroU64::new(1).unwrap(),
            public_salt: vec![value; 32],
            secret_policy: SecretPolicy::Absent,
        },
        profile_id: [51; 32],
        suite_id: SUITE_ID.into(),
    };
    let candidates = Box::new(std::iter::once(Ok(Candidate::new(vec![value; 32])?)));
    let limits = verification_limits();
    let enrolled = enroll_candidates(&ExampleCrypto, candidates, context, None, &limits)?;
    Ok(EnrollmentPackage::from_enrollment(&enrolled, &limits)?)
}

async fn sign(
    chain: &Chain,
    index: usize,
    package: &EnrollmentPackage,
    value: u8,
    transfer: &Transfer,
) -> Result<Authorization> {
    let nonce = chain
        .enrollment(index)
        .await?
        .context("missing enrollment")?
        .next_nonce;
    let action = Action {
        operation: TRANSFER_OPERATION.into(),
        payload: transfer.signing_payload(),
        nonce,
        expires_at_ms: chain.block_time_ms().await? + 600_000,
    };
    let mut limits = verification_limits();
    limits.candidate_attempts = NonZeroUsize::new(1).unwrap();
    let signed = authorization::sign_candidates(
        &ExampleCrypto,
        Box::new(std::iter::once(Ok(Candidate::new(vec![value; 32])?))),
        [51; 32],
        package,
        action.to_core()?,
        None,
        &limits,
    )?;
    Ok(Authorization::from_core(&signed)?)
}

#[tokio::test]
#[ignore = "builds the NEAR wallet/token Wasm and starts a local Sandbox"]
async fn transfers_use_real_balances_and_retain_uncertain_submissions() -> Result<()> {
    let wasm = crate::runtime::prepare_contracts(|_| {}).await?;
    let (mut chain, contexts) = Chain::bootstrap(&wasm).await?;
    let packages = [enrollment(&contexts[0], 71)?, enrollment(&contexts[1], 72)?];
    chain
        .register([
            Commitment::from_core(packages[0].commitment())?,
            Commitment::from_core(packages[1].commitment())?,
        ])
        .await?;
    assert!(chain.transfer_request(2, TOKEN_UNIT, None).is_err());
    assert!(chain.transfer_request(0, 0, None).is_err());
    assert!(
        chain
            .transfer_request(0, TOKEN_UNIT, Some("a".repeat(257)))
            .is_err()
    );
    assert!(!chain.token_id().is_empty());
    assert!(chain.refresh_pending().await?.is_none());

    let transfer = chain.transfer_request(0, TOKEN_UNIT, Some("demo chain test".into()))?;
    let signed = sign(&chain, 0, &packages[0], 71, &transfer).await?;
    assert!(chain.verify(0, &transfer, &signed).await?);
    let mut wrong_token = transfer.clone();
    wrong_token.token_id = contexts[1].wallet_id.parse()?;
    assert!(chain.submit(0, &wrong_token, &signed).await.is_err());
    let mut wrong_receiver = transfer.clone();
    wrong_receiver.receiver_id = contexts[0].wallet_id.parse()?;
    assert!(chain.submit(0, &wrong_receiver, &signed).await.is_err());

    let sent = chain.submit(0, &transfer, &signed).await?;
    assert_eq!(sent.status, SubmissionStatus::Succeeded, "{sent:?}");
    assert!(sent.tx_hash.as_ref().is_some_and(|hash| !hash.is_empty()));
    assert!(
        sent.gas_burnt
            .as_ref()
            .is_some_and(|gas| gas.parse::<u128>().unwrap() > 0)
    );
    assert!(sent.failures.is_empty());
    assert!(sent.logs.iter().any(|log| log.contains("ft_transfer")));
    assert_eq!(
        sent.balances,
        Some([
            (999 * TOKEN_UNIT).to_string(),
            (1001 * TOKEN_UNIT).to_string()
        ])
    );
    assert!(!chain.has_pending());
    assert!(!chain.verify(0, &transfer, &signed).await?);

    let replay = chain.submit(0, &transfer, &signed).await?;
    assert_eq!(replay.status, SubmissionStatus::Rejected, "{replay:?}");
    assert!(!replay.failures.is_empty());
    assert!(!chain.has_pending());
    assert_eq!(replay.balances, sent.balances);

    let transfer = chain.transfer_request(1, 2 * TOKEN_UNIT, None)?;
    let signed = sign(&chain, 1, &packages[1], 72, &transfer).await?;
    let reverse = chain.submit(1, &transfer, &signed).await?;
    assert_eq!(reverse.status, SubmissionStatus::Succeeded, "{reverse:?}");
    assert_eq!(
        reverse.balances,
        Some([
            (1001 * TOKEN_UNIT).to_string(),
            (999 * TOKEN_UNIT).to_string()
        ])
    );

    // A token failure consumes the authorization nonce and returns a real
    // failed callback; it must never be displayed as a successful transfer.
    let excessive = chain.transfer_request(1, INITIAL_BALANCE * 3, None)?;
    let signed = sign(&chain, 1, &packages[1], 72, &excessive).await?;
    let failed = chain.submit(1, &excessive, &signed).await?;
    assert_eq!(failed.status, SubmissionStatus::Failed, "{failed:?}");
    assert_eq!(failed.balances, reverse.balances);
    assert!(!failed.failures.is_empty());
    assert_eq!(chain.enrollment(1).await?.unwrap().next_nonce, 2);
    assert!(!chain.has_pending());

    uncertain_submission_is_never_rebroadcast(&mut chain, &packages[0]).await?;
    Ok(())
}

async fn uncertain_submission_is_never_rebroadcast(
    chain: &mut Chain,
    package: &EnrollmentPackage,
) -> Result<()> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let handler = move |axum::Json(request): axum::Json<serde_json::Value>| {
        observed
            .lock()
            .unwrap()
            .push(request["method"].as_str().unwrap().to_owned());
        async move {
            axum::Json(
                json!({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32603, "message": "injected RPC interruption"}}),
            )
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let rpc = format!("http://{}", listener.local_addr()?);
    let router = axum::Router::new().route("/", axum::routing::post(handler));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let transfer = chain.transfer_request(0, TOKEN_UNIT, None)?;
    let signed = sign(chain, 0, package, 71, &transfer).await?;
    let balances = chain.balances().await?;
    chain.rpc = JsonRpcClient::connect(rpc);
    let unknown = chain.submit(0, &transfer, &signed).await?;
    assert_eq!(unknown.status, SubmissionStatus::Unknown);
    assert!(unknown.tx_hash.is_some());
    assert!(chain.has_pending());
    assert_eq!(
        *calls.lock().unwrap(),
        ["send_tx"],
        "a submission may only broadcast once"
    );
    assert!(chain.submit(0, &transfer, &signed).await.is_err());
    assert_eq!(*calls.lock().unwrap(), ["send_tx"]);
    let refreshed = chain.refresh_pending().await?.unwrap();
    assert_eq!(refreshed.status, SubmissionStatus::Unknown);
    assert_eq!(refreshed.tx_hash, unknown.tx_hash);
    assert_eq!(
        *calls.lock().unwrap(),
        ["send_tx", "tx"],
        "refresh makes one status query"
    );
    assert_eq!(chain.balances().await?, balances);
    assert_eq!(chain.enrollment(0).await?.unwrap().next_nonce, 1);
    server.abort();
    Ok(())
}
