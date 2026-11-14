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
    let sandbox = Sandbox::start(&wasm).await?;
    let (mut chain, contexts) = sandbox.create_session().await?;
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
    let transaction = chain.sign_transaction(0, &transfer, &signed).await?;
    let [TransactionAction::FunctionCall(call)] = transaction.transaction.actions() else {
        panic!("expected a single transfer function call");
    };
    assert_eq!(call.method_name, "transfer");
    assert_eq!(call.gas.as_gas(), 100_000_000_000_000);
    let mut wrong_token = transfer.clone();
    wrong_token.token_id = contexts[1].wallet_id.parse()?;
    assert!(chain.submit(0, &wrong_token, &signed).await.is_err());
    let mut wrong_receiver = transfer.clone();
    wrong_receiver.receiver_id = contexts[0].wallet_id.parse()?;
    assert!(chain.submit(0, &wrong_receiver, &signed).await.is_err());

    let sent = chain.submit(0, &transfer, &signed).await?;
    assert_eq!(sent.status, SubmissionStatus::Succeeded, "{sent:?}");
    assert!(sent.tx_hash.as_ref().is_some_and(|hash| !hash.is_empty()));
    assert!(sent.gas_burnt.as_ref().is_some_and(|gas| {
        let gas = gas.parse::<u128>().unwrap();
        gas > 0 && gas <= u128::from(call.gas.as_gas())
    }));
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
    chain.close().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "builds the NEAR contracts and proves two sessions share one actual Sandbox"]
async fn visitor_sessions_isolate_contracts_balances_signatures_and_cleanup() -> Result<()> {
    let wasm = crate::runtime::prepare_contracts(|_| {}).await?;
    let sandbox = Sandbox::start(&wasm).await?;
    let (first, second) = tokio::join!(sandbox.create_session(), sandbox.create_session());
    let (mut first, first_contexts) = first?;
    let (mut second, second_contexts) = second?;
    assert_eq!(first.worker.rpc_addr(), second.worker.rpc_addr());
    assert_eq!(first.chain_id, second.chain_id);
    assert_ne!(first.token_id(), second.token_id());
    assert_ne!(first.relayer_id(), second.relayer_id());
    for (a, b) in first_contexts.iter().zip(&second_contexts) {
        assert_ne!(a.wallet_id, b.wallet_id);
    }

    let first_packages = [
        enrollment(&first_contexts[0], 81)?,
        enrollment(&first_contexts[1], 82)?,
    ];
    let second_packages = [
        enrollment(&second_contexts[0], 81)?,
        enrollment(&second_contexts[1], 82)?,
    ];
    assert_ne!(
        first_packages[0].commitment().root,
        second_packages[0].commitment().root
    );
    assert!(
        second
            .register([
                Commitment::from_core(first_packages[0].commitment())?,
                Commitment::from_core(first_packages[1].commitment())?,
            ])
            .await
            .is_err()
    );
    for (chain, packages) in [(&first, &first_packages), (&second, &second_packages)] {
        chain
            .register([
                Commitment::from_core(packages[0].commitment())?,
                Commitment::from_core(packages[1].commitment())?,
            ])
            .await?;
    }

    let transfer = first.transfer_request(0, 10 * TOKEN_UNIT, None)?;
    let signature = sign(&first, 0, &first_packages[0], 81, &transfer).await?;
    let other_transfer = second.transfer_request(0, 10 * TOKEN_UNIT, None)?;
    assert!(!second.verify(0, &other_transfer, &signature).await?);
    assert!(second.submit(0, &transfer, &signature).await.is_err());
    let result = first.submit(0, &transfer, &signature).await?;
    assert_eq!(result.status, SubmissionStatus::Succeeded, "{result:?}");
    assert_eq!(
        first.balances().await?,
        [
            (990 * TOKEN_UNIT).to_string(),
            (1010 * TOKEN_UNIT).to_string(),
        ]
    );
    assert_eq!(
        second.balances().await?,
        [INITIAL_BALANCE.to_string(), INITIAL_BALANCE.to_string()]
    );
    assert_eq!(first.enrollment(0).await?.unwrap().next_nonce, 1);
    assert_eq!(second.enrollment(0).await?.unwrap().next_nonce, 0);

    let first_ids = session_account_ids(&first)?;
    first.close().await?;
    first.close().await?;
    for id in &first_ids {
        assert_account_deleted(&second.rpc, id).await?;
    }
    // The other visitor can still sign and execute after the first is deleted.
    let signature = sign(&second, 0, &second_packages[0], 81, &other_transfer).await?;
    let result = second.submit(0, &other_transfer, &signature).await?;
    assert_eq!(result.status, SubmissionStatus::Succeeded, "{result:?}");

    let second_ids = session_account_ids(&second)?;
    second.close().await?;
    for id in &second_ids {
        assert_account_deleted(&first.rpc, id).await?;
    }
    first.worker.status().await?;
    Ok(())
}

fn session_account_ids(chain: &Chain) -> Result<Vec<near_sdk::AccountId>> {
    let wallet = chain.wallets[0].id().as_str();
    let parent = wallet
        .strip_prefix("wallet-1.")
        .context("missing session parent")?;
    Ok(vec![
        chain.wallets[0].id().clone(),
        chain.wallets[1].id().clone(),
        chain.token.id().clone(),
        chain.relayer.id().clone(),
        parent.parse()?,
    ])
}

#[tokio::test]
#[ignore = "starts a real Sandbox and cancels an accepted transfer before its RPC reply"]
async fn retirement_settles_a_broadcast_after_its_caller_is_cancelled() -> Result<()> {
    let wasm = crate::runtime::prepare_contracts(|_| {}).await?;
    let sandbox = Sandbox::start(&wasm).await?;
    let (mut chain, contexts) = sandbox.create_session().await?;
    let packages = [enrollment(&contexts[0], 91)?, enrollment(&contexts[1], 92)?];
    chain
        .register([
            Commitment::from_core(packages[0].commitment())?,
            Commitment::from_core(packages[1].commitment())?,
        ])
        .await?;
    let transfer = chain.transfer_request(0, TOKEN_UNIT, None)?;
    let signed = sign(&chain, 0, &packages[0], 91, &transfer).await?;
    let accepted = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let rpc = JsonRpcClient::connect(chain.worker.rpc_addr());
    let handler = {
        let accepted = Arc::clone(&accepted);
        let release = Arc::clone(&release);
        move |axum::Json(request): axum::Json<serde_json::Value>| {
            let rpc = rpc.clone();
            let accepted = Arc::clone(&accepted);
            let release = Arc::clone(&release);
            async move {
                assert_eq!(request["method"], "send_tx");
                let submission: methods::send_tx::RpcSendTransactionRequest =
                    serde_json::from_value(request["params"].clone()).unwrap();
                let response = rpc.call(submission).await.unwrap();
                // The chain accepted the transaction, but the session has not
                // received that fact when its expiry aborts the submit future.
                accepted.notify_one();
                release.notified().await;
                axum::Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":response}))
            }
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    chain.rpc = JsonRpcClient::connect(format!("http://{}", listener.local_addr()?));
    let router = axum::Router::new().route("/", axum::routing::post(handler));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    {
        let submission = chain.submit(0, &transfer, &signed);
        tokio::pin!(submission);
        tokio::select! {
            () = accepted.notified() => {},
            result = &mut submission => bail!("submission completed before cancellation: {result:?}"),
        }
    }
    assert!(chain.has_pending());
    release.notify_one();
    let ids = session_account_ids(&chain)?;
    chain.close().await?;
    let rpc = JsonRpcClient::connect(chain.worker.rpc_addr());
    for id in &ids {
        assert_account_deleted(&rpc, id).await?;
    }
    chain.worker.status().await?;
    server.abort();
    Ok(())
}

async fn assert_account_deleted(rpc: &JsonRpcClient, id: &near_sdk::AccountId) -> Result<()> {
    let result = rpc
        .call(methods::query::RpcQueryRequest {
            block_reference: near_primitives::types::BlockReference::latest(),
            request: near_primitives::views::QueryRequest::ViewAccount {
                account_id: id.clone(),
            },
        })
        .await;
    assert!(
        matches!(
            result,
            Err(JsonRpcError::ServerError(JsonRpcServerError::HandlerError(
                methods::query::RpcQueryError::UnknownAccount { .. },
            )))
        ),
        "account {id} was not confirmed deleted: {result:?}"
    );
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
