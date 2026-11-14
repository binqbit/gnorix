//! Actual circular samples through the application and deployed wallets.
//! Fixed seeds make independent attempts repeatable, without runtime retries.

use std::time::Duration;

use tokio::time::{sleep, timeout};

use super::*;
use crate::model::TOKEN_UNIT;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "builds contracts, analyzes both bundled fingerprint images and runs a local Sandbox"]
async fn fingerprint_approvals_execute_real_transfers_between_both_wallets() -> Result<()> {
    let app = App::new(8780)?;
    app.start();
    let outcome = timeout(Duration::from_secs(900), exercise_application(&app)).await;
    // Keep cleanup outside the scenario Result so failed assertions/timeouts
    // still stop the managed node and cancel any unfinished application task.
    timeout(Duration::from_secs(10), app.stop())
        .await
        .context("application shutdown timed out")?;
    ensure!(
        app.engine.lock().await.is_none(),
        "shutdown retained the chain"
    );
    outcome.context("fingerprint acceptance scenario timed out")?
}

async fn exercise_application(app: &App) -> Result<()> {
    let ready = wait_for_phase(app, "ready").await?;
    ensure!(ready.wallets.len() == 2);
    ensure!(ready.wallets[0].account_id != ready.wallets[1].account_id);
    ensure!(ready.wallets[0].root != ready.wallets[1].root);
    check_balances_and_nonces(&ready, [1000 * TOKEN_UNIT; 2], [0, 0])?;
    for wallet in &ready.wallets {
        ensure!(wallet.leaf_count > 0 && wallet.root.len() == 64);
        ensure!(wallet.fingerprint.feature_count >= 3);
        ensure!(wallet.fingerprint.feature_count == wallet.fingerprint.points.len());
        ensure!(
            wallet
                .fingerprint
                .points
                .iter()
                .all(|point| point.priority >= 0.1)
        );
        println!(
            "{} ready: {} eligible features, {} leaves",
            wallet.name, wallet.fingerprint.feature_count, wallet.leaf_count
        );
    }

    // These seeds were checked independently under the production policy. Each
    // appears once; a failed crop is asserted as failure, never retried silently.
    let failed_capture = failed_attempt_is_retained(app).await?;

    let forward = TOKEN_UNIT * 125 / 100;
    let reverse = TOKEN_UNIT * 25 / 10;
    let steps = [
        (
            0,
            4_u64,
            "1.25",
            forward,
            [1000 * TOKEN_UNIT - forward, 1000 * TOKEN_UNIT + forward],
            [1, 0],
        ),
        (
            1,
            1_u64,
            "2.5",
            reverse,
            [
                1000 * TOKEN_UNIT - forward + reverse,
                1000 * TOKEN_UNIT + forward - reverse,
            ],
            [1, 1],
        ),
    ];
    let mut previous_request = Some(failed_capture);
    for (wallet, seed, amount, units, balances, nonces) in steps {
        let before = app.snapshot();
        app.launch(Command::Sign(
            SignRequest {
                wallet,
                recipient: 1 - wallet,
                amount: amount.into(),
                memo: Some("Fingerprint service acceptance".into()),
            },
            seed,
        ))?;
        if let Some(previous) = &previous_request {
            ensure!(
                app.capture_image(previous).is_none(),
                "old scan remained addressable after a new attempt"
            );
        }
        let signed = wait_for_phase(app, "signed").await?;
        check_capture(app, &signed, wallet)?;
        let capture = signed
            .capture
            .as_ref()
            .context("signed state has no capture")?;
        let prepared = signed
            .prepared
            .as_ref()
            .context("signed state has no authorization")?;
        ensure!(prepared.verified && prepared.wallet == wallet && prepared.recipient == 1 - wallet);
        ensure!(prepared.amount_base_units == units.to_string() && prepared.attempts > 0);
        ensure!(prepared.capture_id == capture.id && prepared.id == capture.id);
        ensure!(
            previous_request.as_ref() != Some(&capture.id),
            "new action reused a capture identifier"
        );
        ensure!(capture.matched_feature_ids == prepared.matched_feature_ids);
        ensure!((3..=6).contains(&prepared.matched_feature_ids.len()));
        ensure!(
            prepared
                .matched_feature_ids
                .iter()
                .all(|id| { capture.points.iter().any(|point| point.id == *id) })
        );
        for (after, before) in signed.wallets.iter().zip(&before.wallets) {
            ensure!(
                after.balance_base_units == before.balance_base_units
                    && after.nonce == before.nonce,
                "signing changed on-chain balances or consumed a nonce"
            );
        }
        verify_exact_prepared_request(app, prepared, units).await?;
        let stale = previous_request
            .clone()
            .unwrap_or_else(|| "unknown-request".into());
        ensure!(
            app.submit(SubmitRequest { prepared_id: stale }).is_err(),
            "stale request was accepted"
        );
        ensure!(
            app.snapshot().phase == "signed",
            "stale request discarded the valid approval"
        );

        app.submit(SubmitRequest {
            prepared_id: prepared.id.clone(),
        })?;
        ensure!(
            app.submit(SubmitRequest {
                prepared_id: prepared.id.clone()
            })
            .is_err(),
            "duplicate submission was accepted"
        );
        let complete = wait_for_phase(app, "complete").await?;
        let result = complete
            .transaction
            .as_ref()
            .context("completed state has no transaction")?;
        ensure!(
            result.status == SubmissionStatus::Succeeded,
            "transfer did not succeed: {result:?}"
        );
        ensure!(result.tx_hash.as_ref().is_some_and(|hash| !hash.is_empty()));
        ensure!(result.callback.as_ref().is_some_and(
            |callback| callback.status == gnorix_near_wallet::TransferStatus::Succeeded
        ));
        ensure!(result.failures.is_empty() && result.observation_error.is_none());
        ensure!(result.balances.as_ref() == Some(&balances.map(|value| value.to_string())));
        check_balances_and_nonces(&complete, balances, nonces)?;
        verify_consumed_request(app, prepared, units).await?;
        ensure!(
            app.submit(SubmitRequest {
                prepared_id: prepared.id.clone()
            })
            .is_err()
        );
        println!(
            "Wallet {} sent {amount} GXT: {} matched features, {} attempts, transaction {}",
            wallet + 1,
            prepared.matched_feature_ids.len(),
            prepared.attempts,
            result.tx_hash.as_deref().unwrap_or_default()
        );
        previous_request = Some(prepared.id.clone());
    }
    Ok(())
}

async fn failed_attempt_is_retained(app: &App) -> Result<String> {
    app.launch(Command::Sign(
        SignRequest {
            wallet: 0,
            recipient: 1,
            amount: "1.25".into(),
            memo: None,
        },
        2,
    ))?;
    let failed = wait_for_phase(app, "failed").await?;
    ensure!(failed.prepared.is_none() && failed.transaction.is_none());
    ensure!(
        failed.error.as_deref()
            == Some("This capture did not reconstruct an enrolled key. Try a new random capture."),
        "expected the known crop's unsuccessful reconstruction"
    );
    check_capture(app, &failed, 0)?;
    check_balances_and_nonces(&failed, [1000 * TOKEN_UNIT; 2], [0, 0])?;
    let capture = failed.capture.as_ref().context("failed scan disappeared")?;
    ensure!(capture.id == "1" && capture.matched_feature_ids.is_empty());
    {
        let guard = app.engine.lock().await;
        let engine = guard.as_ref().context("missing engine")?;
        ensure!(engine.prepared.is_none() && !engine.chain.has_pending());
        ensure!(
            engine.chain.balances().await?
                == [
                    (1000 * TOKEN_UNIT).to_string(),
                    (1000 * TOKEN_UNIT).to_string()
                ]
        );
        for index in 0..2 {
            ensure!(
                engine
                    .chain
                    .enrollment(index)
                    .await?
                    .context("missing enrollment")?
                    .next_nonce
                    == 0
            );
        }
    }
    let id = capture.id.clone();
    let bytes = app
        .capture_image(&id)
        .context("failed capture PNG is unavailable")?;
    sleep(Duration::from_millis(200)).await;
    ensure!(
        !app.snapshot().busy
            && app
                .snapshot()
                .capture
                .as_ref()
                .is_some_and(|capture| capture.id == id)
    );
    ensure!(
        app.capture_image(&id).as_deref() == Some(bytes.as_ref()),
        "failed capture changed without another action"
    );
    println!(
        "Failed capture {id} retained without a transaction: {}",
        failed.error.as_deref().unwrap_or_default()
    );
    Ok(id)
}

fn check_capture(app: &App, state: &Snapshot, wallet: usize) -> Result<()> {
    let capture = state
        .capture
        .as_ref()
        .context("attempt has no visible capture")?;
    ensure!(capture.wallet == wallet);
    ensure!((1.0 / 3.0..=1.0).contains(&capture.pattern_fraction));
    ensure!(capture.region.radius.is_finite() && capture.region.radius >= 8.0);
    ensure!(capture.points.iter().all(|point| point.priority >= 0.1));
    if let Some(count) = capture.feature_count {
        ensure!(count == capture.points.len());
    } else {
        ensure!(capture.points.is_empty());
    }
    ensure!(capture.points.iter().all(|point| {
        (point.x - capture.region.x).hypot(point.y - capture.region.y)
            <= capture.region.radius + 1e-6
    }));
    let png = app
        .capture_image(&capture.id)
        .context("capture preview is unavailable")?;
    let preview = image::load_from_memory(&png)?.to_luma8();
    ensure!(preview.dimensions() == (capture.width, capture.height));
    let original = image::load_from_memory(
        crate::fingerprint::image(wallet).context("missing source image")?,
    )?
    .to_luma8();
    ensure!(
        preview == original,
        "capture preview must preserve the source pixels; blur is presentation-only"
    );
    Ok(())
}

async fn verify_exact_prepared_request(app: &App, view: &PreparedView, units: u128) -> Result<()> {
    let guard = app.engine.lock().await;
    let engine = guard.as_ref().context("missing engine")?;
    let prepared = engine
        .prepared
        .as_ref()
        .context("missing retained approval")?;
    let expected = engine
        .chain
        .transfer_request(view.wallet, units, view.memo.clone())?;
    ensure!(prepared.transfer == expected && prepared.authorization == view.authorization);
    ensure!(view.authorization.action.operation == TRANSFER_OPERATION);
    ensure!(view.authorization.action.payload == expected.signing_payload());
    ensure!(
        engine
            .chain
            .verify(view.wallet, &expected, &view.authorization)
            .await?
    );

    let mut altered_transfer = expected.clone();
    altered_transfer.amount.0 += 1;
    ensure!(
        !engine
            .chain
            .verify(view.wallet, &altered_transfer, &view.authorization)
            .await?,
        "changed amount passed the contract verifier"
    );
    let mut altered_proof = view.authorization.clone();
    altered_proof.action.payload.push(0);
    ensure!(
        !engine
            .chain
            .verify(view.wallet, &expected, &altered_proof)
            .await?,
        "changed signed payload passed the contract verifier"
    );
    ensure!(
        engine
            .chain
            .verify(view.wallet, &expected, &view.authorization)
            .await?,
        "read-only checks consumed the nonce"
    );
    Ok(())
}

async fn verify_consumed_request(app: &App, view: &PreparedView, units: u128) -> Result<()> {
    let guard = app.engine.lock().await;
    let engine = guard.as_ref().context("missing engine")?;
    let transfer = engine
        .chain
        .transfer_request(view.wallet, units, view.memo.clone())?;
    ensure!(
        !engine
            .chain
            .verify(view.wallet, &transfer, &view.authorization)
            .await?,
        "executed authorization remained valid"
    );
    Ok(())
}

fn check_balances_and_nonces(
    snapshot: &Snapshot,
    balances: [u128; 2],
    nonces: [u64; 2],
) -> Result<()> {
    for (index, wallet) in snapshot.wallets.iter().enumerate() {
        ensure!(
            wallet.balance_base_units == balances[index].to_string(),
            "wrong balance for wallet {index}"
        );
        ensure!(
            wallet.nonce == nonces[index].to_string(),
            "wrong nonce for wallet {index}"
        );
    }
    Ok(())
}

async fn wait_for_phase(app: &App, phase: &str) -> Result<Snapshot> {
    let mut last_message = String::new();
    loop {
        let state = app.snapshot();
        if state.message != last_message {
            println!("{}: {}", state.phase, state.message);
            last_message.clone_from(&state.message);
        }
        if !state.busy {
            if phase == "complete" && state.phase == "unknown" {
                // A status query is safe if receipts are slow. Never re-sign or
                // resubmit the already accepted request to make this test pass.
                app.refresh()?;
            } else {
                ensure!(
                    state.phase == phase,
                    "expected {phase}, got {}: {}",
                    state.phase,
                    state.error.as_deref().unwrap_or(&state.message)
                );
                return Ok(state);
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
}
