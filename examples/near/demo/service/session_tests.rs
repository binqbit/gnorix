//! Real browser-session isolation through fresh enrollment and deployed contracts.

use std::{
    collections::HashSet,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use tokio::time::{sleep, timeout};

use super::*;
use crate::{
    config::Config,
    model::TOKEN_UNIT,
    sessions::{Demo, SessionError},
};

struct BrowserSession {
    session: Session,
    cookie: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "builds contracts, enrolls independent five-point sessions and executes a real Sandbox transfer"]
async fn browser_sessions_isolate_signing_transfers_and_expiry() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let demo = Demo::new(Config::parse([
        "--port".into(),
        address.port().to_string(),
        "--points-per-combination".into(),
        "5".into(),
    ])?)?;
    let routes = crate::api::router(demo.clone());
    let server = tokio::spawn(async move { axum::serve(listener, routes).await });
    demo.start();

    let outcome = timeout(Duration::from_secs(900), exercise_sessions(&demo, address)).await;
    // An assertion error or timeout must not leave another managed node running.
    let shutdown = timeout(Duration::from_secs(60), demo.stop()).await;
    server.abort();
    let _ = server.await;
    shutdown.context("isolated-session shutdown timed out")?;
    outcome.context("isolated-session acceptance test timed out")?
}

async fn exercise_sessions(demo: &Demo, address: SocketAddr) -> Result<()> {
    let a = open_browser_session(demo, address, None).await?;
    let b = open_browser_session(demo, address, None).await?;
    let ready_a = wait_idle(&a.session, "ready").await?;
    let ready_b = wait_idle(&b.session, "ready").await?;
    ensure!(a.cookie != b.cookie && ready_a.token != ready_b.token);
    check_distinct_enrollments(&[&ready_a, &ready_b])?;
    check_balances(&ready_a, [1000 * TOKEN_UNIT; 2], [0, 0])?;
    check_balances(&ready_b, [1000 * TOKEN_UNIT; 2], [0, 0])?;
    let untouched_b = serde_json::to_value(&ready_b)?;
    println!("Two HTTP sessions enrolled independent five-point wallet roots and token contracts");

    // One known valid five-point crop, once. A failed crop fails this test;
    // neither this test nor the service silently retries another sample.
    a.session.launch(Command::Sign(
        SignRequest {
            wallet: 1,
            recipient: 0,
            amount: "1".into(),
            memo: Some("Independent browser-session acceptance".into()),
        },
        118_278,
    ))?;
    let signed = wait_idle(&a.session, "signed").await?;
    let capture = signed.capture.as_ref().context("missing actual capture")?;
    let prepared = signed.prepared.as_ref().context("missing authorization")?;
    ensure!(prepared.verified && prepared.matched_feature_ids.len() == 5);
    ensure!(capture.matched_feature_ids == prepared.matched_feature_ids);
    ensure!(prepared.wallet == 1 && prepared.recipient == 0);
    ensure!(prepared.amount_base_units == TOKEN_UNIT.to_string());
    ensure!(prepared.attempts > 0 && capture.pattern_fraction >= 1.0 / 3.0);
    ensure!(
        prepared
            .matched_feature_ids
            .iter()
            .all(|id| { capture.points.iter().any(|point| point.id == *id) })
    );
    ensure!(a.session.capture_image(&capture.id).is_some());
    ensure!(b.session.capture_image(&capture.id).is_none());
    ensure!(serde_json::to_value(b.session.snapshot())? == untouched_b);
    reject_cross_session_authorization(&a.session, &b.session, prepared).await?;

    a.session.submit(SubmitRequest {
        prepared_id: prepared.id.clone(),
    })?;
    let complete = wait_idle(&a.session, "complete").await?;
    let transaction = complete
        .transaction
        .as_ref()
        .context("missing transfer result")?;
    ensure!(transaction.status == SubmissionStatus::Succeeded);
    ensure!(
        transaction
            .tx_hash
            .as_ref()
            .is_some_and(|hash| !hash.is_empty())
    );
    ensure!(transaction.failures.is_empty() && transaction.observation_error.is_none());
    check_balances(&complete, [1001 * TOKEN_UNIT, 999 * TOKEN_UNIT], [0, 1])?;
    ensure!(serde_json::to_value(b.session.snapshot())? == untouched_b);
    check_b_on_chain(&b.session, &ready_b).await?;
    println!(
        "A transferred 1 GXT with five fingerprint features; B rejected A's authorization and retained its balances/nonces: {}",
        transaction.tx_hash.as_deref().unwrap_or_default()
    );

    demo.expire_for_test(a.session.id());
    ensure!(matches!(
        demo.existing_session(a.session.id()),
        Err(SessionError::Expired)
    ));
    ensure!(
        a.session
            .submit(SubmitRequest {
                prepared_id: prepared.id.clone()
            })
            .is_err()
    );
    timeout(Duration::from_secs(30), wait_for_disposal(demo, &a.session))
        .await
        .context("expired session retained application state")?;
    ensure!(a.session.capture_image(&capture.id).is_none());
    check_b_on_chain(&b.session, &ready_b).await?;
    ensure!(demo.existing_session(b.session.id()).is_ok());
    println!("A expired and released its session state; B still refreshes its deployed wallets");

    let replacement = open_browser_session(demo, address, Some(&a.cookie)).await?;
    ensure!(replacement.cookie != a.cookie && replacement.session.id() != a.session.id());
    let ready_replacement = wait_idle(&replacement.session, "ready").await?;
    check_distinct_enrollments(&[&ready_a, &ready_b, &ready_replacement])?;
    check_balances(&ready_replacement, [1000 * TOKEN_UNIT; 2], [0, 0])?;
    check_b_on_chain(&b.session, &ready_b).await?;
    ensure!(demo.session_count() == 2);
    println!("GET /api/state renewed the expired browser with new contracts and new Merkle roots");
    Ok(())
}

async fn reject_cross_session_authorization(
    a: &Session,
    b: &Session,
    view: &PreparedView,
) -> Result<()> {
    let transfer_a = {
        let guard = a.engine.lock().await;
        let engine = guard.as_ref().context("A engine is missing")?;
        let expected = engine
            .chain
            .transfer_request(1, TOKEN_UNIT, view.memo.clone())?;
        ensure!(view.authorization.action.payload == expected.signing_payload());
        ensure!(
            engine
                .chain
                .verify(1, &expected, &view.authorization)
                .await?
        );
        expected
    };
    let guard = b.engine.lock().await;
    let engine = guard.as_ref().context("B engine is missing")?;
    let transfer_b = engine
        .chain
        .transfer_request(1, TOKEN_UNIT, view.memo.clone())?;
    ensure!(transfer_a != transfer_b);
    ensure!(
        !engine
            .chain
            .verify(1, &transfer_b, &view.authorization)
            .await?,
        "B accepted A's copied authorization for B's expected transfer"
    );
    Ok(())
}

async fn check_b_on_chain(session: &Session, original: &Snapshot) -> Result<()> {
    session.refresh()?;
    let refreshed = wait_idle(session, "ready").await?;
    check_balances(&refreshed, [1000 * TOKEN_UNIT; 2], [0, 0])?;
    ensure!(
        serde_json::to_value(&refreshed.wallets)? == serde_json::to_value(&original.wallets)?,
        "another visitor changed B's wallet state"
    );
    ensure!(
        refreshed.prepared.is_none()
            && refreshed.capture.is_none()
            && refreshed.transaction.is_none()
    );
    Ok(())
}

fn check_distinct_enrollments(snapshots: &[&Snapshot]) -> Result<()> {
    let mut accounts = HashSet::new();
    let mut roots = HashSet::new();
    let mut tokens = HashSet::new();
    for snapshot in snapshots {
        ensure!(snapshot.wallets.len() == 2);
        ensure!(
            tokens.insert(
                snapshot
                    .token_id
                    .as_ref()
                    .context("missing token contract")?
            )
        );
        for wallet in &snapshot.wallets {
            ensure!(
                accounts.insert(&wallet.account_id),
                "wallet contract was shared across sessions"
            );
            ensure!(
                roots.insert(&wallet.root),
                "Merkle root was shared across sessions"
            );
            ensure!(wallet.leaf_count > 0 && wallet.root.len() == 64);
        }
    }
    Ok(())
}

fn check_balances(snapshot: &Snapshot, balances: [u128; 2], nonces: [u64; 2]) -> Result<()> {
    ensure!(snapshot.wallets.len() == 2);
    for (index, wallet) in snapshot.wallets.iter().enumerate() {
        ensure!(
            wallet.balance_base_units == balances[index].to_string(),
            "incorrect balance for wallet {index}"
        );
        ensure!(
            wallet.nonce == nonces[index].to_string(),
            "incorrect nonce for wallet {index}"
        );
    }
    Ok(())
}

async fn wait_idle(session: &Session, phase: &str) -> Result<Snapshot> {
    loop {
        let state = session.snapshot();
        if !state.busy {
            ensure!(
                state.phase == phase,
                "expected {phase}, got {}: {} {:?}",
                state.phase,
                state.message,
                state.error
            );
            return Ok(state);
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_disposal(demo: &Demo, session: &Session) {
    loop {
        let state = session.snapshot();
        if demo.session_count() == 1
            && state.wallets.is_empty()
            && state.prepared.is_none()
            && state.capture.is_none()
            && state.transaction.is_none()
            && session.engine.lock().await.is_none()
        {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn open_browser_session(
    demo: &Demo,
    address: SocketAddr,
    cookie: Option<&str>,
) -> Result<BrowserSession> {
    let cookie_header = cookie
        .map(|value| format!("Cookie: {value}\r\n"))
        .unwrap_or_default();
    let response = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        write!(
            stream,
            "GET /api/state HTTP/1.1\r\nHost: {address}\r\n{cookie_header}Connection: close\r\n\r\n"
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    })
    .await??;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .context("invalid HTTP response")?;
    ensure!(
        headers
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("HTTP/1.1 200 ")),
        "session request failed: {response}"
    );
    let cookie = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, value)| {
            value
                .trim()
                .split(';')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .context("new browser session has no cookie")?;
    let (_, id) = cookie.split_once('=').context("invalid session cookie")?;
    let session = demo
        .existing_session(id)
        .map_err(|error| anyhow::anyhow!("session is unavailable: {error:?}"))?;
    let snapshot: serde_json::Value = serde_json::from_str(body)?;
    ensure!(snapshot["token"] == session.snapshot().token);
    Ok(BrowserSession { session, cookie })
}
