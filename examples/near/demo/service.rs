use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, ensure};
use gnorix::near::{Action, Authorization};
use gnorix_near_wallet::{TRANSFER_OPERATION, Transfer};
use tokio::{sync::Mutex as AsyncMutex, task::JoinHandle};

use super::{
    chain::{Chain, SubmissionStatus, TransferResult},
    fingerprint::FingerprintWallet,
    model::{
        CaptureView, PreparedView, SignRequest, Snapshot, SubmitRequest, TOKEN_DECIMALS,
        WalletView, format_amount, hex,
    },
    runtime,
};

#[derive(Clone)]
pub struct App {
    pub port: u16,
    state: Arc<Mutex<Snapshot>>,
    engine: Arc<AsyncMutex<Option<Engine>>>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
    preview: Arc<Mutex<Option<CapturePreview>>>,
}

struct Engine {
    chain: Chain,
    wallets: [Arc<FingerprintWallet>; 2],
    prepared: Option<Prepared>,
    sequence: u64,
}

struct Prepared {
    id: String,
    wallet: usize,
    transfer: Transfer,
    authorization: Authorization,
}

struct CapturePreview {
    id: String,
    png: Arc<[u8]>,
}

enum Command {
    Sign(SignRequest, u64),
    Submit(SubmitRequest),
    Refresh,
}

impl App {
    pub fn new(port: u16) -> Result<Self> {
        let mut token = [0_u8; 32];
        getrandom::fill(&mut token).context("create local session token")?;
        Ok(Self {
            port,
            state: Arc::new(Mutex::new(Snapshot {
                token: hex(&token),
                phase: "starting",
                message: "Preparing the local NEAR environment".into(),
                error: None,
                busy: true,
                chain_id: None,
                token_id: None,
                symbol: "GXT",
                decimals: TOKEN_DECIMALS,
                wallets: Vec::new(),
                prepared: None,
                capture: None,
                transaction: None,
            })),
            engine: Arc::new(AsyncMutex::new(None)),
            task: Arc::new(Mutex::new(None)),
            preview: Arc::new(Mutex::new(None)),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("demo state lock poisoned").clone()
    }

    pub fn capture_image(&self, id: &str) -> Option<Arc<[u8]>> {
        let preview = self.preview.lock().expect("capture preview lock poisoned");
        preview
            .as_ref()
            .filter(|preview| preview.id == id)
            .map(|preview| Arc::clone(&preview.png))
    }

    pub fn start(&self) {
        let app = self.clone();
        self.track(tokio::spawn(async move {
            if let Err(error) = app.bootstrap().await {
                app.fail(error);
            }
        }));
    }

    pub async fn stop(&self) {
        let task = self.task.lock().expect("demo task lock poisoned").take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        // Dropping the worker stops the sandbox and disposes its test accounts.
        self.engine.lock().await.take();
    }

    pub fn sign(&self, request: SignRequest) -> Result<()> {
        request.validate()?;
        let mut seed = [0_u8; 8];
        getrandom::fill(&mut seed).context("select a new random capture")?;
        self.launch(Command::Sign(request, u64::from_le_bytes(seed)))
    }

    pub fn submit(&self, request: SubmitRequest) -> Result<()> {
        self.launch(Command::Submit(request))
    }

    pub fn refresh(&self) -> Result<()> {
        self.launch(Command::Refresh)
    }

    fn launch(&self, command: Command) -> Result<()> {
        {
            let mut state = self.state.lock().expect("demo state lock poisoned");
            ensure!(!state.busy, "another operation is still running");
            ensure!(state.wallets.len() == 2, "local environment is not ready");
            match &command {
                Command::Sign(_, _) => {
                    ensure!(
                        !state.transaction.as_ref().is_some_and(unresolved),
                        "check the pending transaction before signing another transfer"
                    );
                    state.phase = "signing";
                    state.prepared = None;
                    state.capture = None;
                    self.preview
                        .lock()
                        .expect("capture preview lock poisoned")
                        .take();
                    state.transaction = None;
                    state.message = "Analyzing the selected test fingerprint".into();
                }
                Command::Submit(request) => {
                    ensure!(
                        state.phase == "signed"
                            && state
                                .prepared
                                .as_ref()
                                .is_some_and(|prepared| prepared.id == request.prepared_id),
                        "the signed transfer is stale or has already been submitted"
                    );
                    state.phase = "submitting";
                    state.message = "Submitting the signed authorization to NEAR".into();
                }
                Command::Refresh => {
                    state.message = if state.transaction.as_ref().is_some_and(unresolved) {
                        "Checking the original transaction; no new transaction is sent"
                    } else {
                        "Refreshing balances and nonces from the local contracts"
                    }
                    .into();
                }
            }
            state.busy = true;
            state.error = None;

            // Publish the owned task before another request can observe idle
            // state, including when a short refresh completes immediately.
            let app = self.clone();
            self.track(tokio::spawn(async move {
                if let Err(error) = app.execute(command).await {
                    app.fail(error);
                }
            }));
        }
        Ok(())
    }

    fn track(&self, task: JoinHandle<()>) {
        *self.task.lock().expect("demo task lock poisoned") = Some(task);
    }

    fn progress(&self, message: &str) {
        eprintln!("{message}");
        self.state.lock().expect("demo state lock poisoned").message = message.into();
    }

    fn fail(&self, error: anyhow::Error) {
        let message = format!("{error:#}");
        eprintln!("Demo operation failed: {message}");
        let mut state = self.state.lock().expect("demo state lock poisoned");
        let reason = if state.phase == "signing" && state.capture.is_some() {
            super::fingerprint::attempt_failure(&error)
        } else {
            None
        };
        state.phase = "failed";
        state.message = reason.unwrap_or("The operation did not complete").into();
        state.error = Some(reason.map(str::to_owned).unwrap_or(message));
        state.busy = false;
    }

    async fn bootstrap(&self) -> Result<()> {
        let app = self.clone();
        let wasm = runtime::prepare_contracts(|message| app.progress(message)).await?;
        self.progress("Starting NEAR sandbox and deploying both wallets and the token");
        let (chain, contexts) = Chain::bootstrap(&wasm).await?;
        let mut wallets = Vec::with_capacity(2);
        for (index, context) in contexts.iter().enumerate() {
            self.progress(&format!(
                "Extracting fingerprint features and enrolling Wallet {}",
                index + 1
            ));
            let context = context.clone();
            let wallet = tokio::task::spawn_blocking(move || {
                FingerprintWallet::enroll(
                    index,
                    &format!("near:{}", context.chain_id),
                    &context.wallet_id,
                )
            })
            .await??;
            wallets.push(Arc::new(wallet));
        }
        let wallets: [Arc<FingerprintWallet>; 2] = wallets
            .try_into()
            .map_err(|_| anyhow::anyhow!("expected two fingerprint wallets"))?;
        self.progress("Registering both fingerprint Merkle roots on the wallet contracts");
        let registrations = chain
            .register([wallets[0].commitment()?, wallets[1].commitment()?])
            .await?;
        let balances = chain.balances().await?;
        let views = (0..2)
            .map(|index| WalletView {
                id: index,
                name: format!("Wallet {}", index + 1),
                account_id: contexts[index].wallet_id.clone(),
                balance_base_units: balances[index].clone(),
                root: hex(&registrations[index].commitment.root),
                nonce: registrations[index].next_nonce.to_string(),
                leaf_count: registrations[index].commitment.leaf_count,
                fingerprint: wallets[index].view(),
            })
            .collect();
        let token_id = chain.token_id().to_owned();
        *self.engine.lock().await = Some(Engine {
            chain,
            wallets,
            prepared: None,
            sequence: 0,
        });

        let mut state = self.state.lock().expect("demo state lock poisoned");
        state.phase = "ready";
        state.message = "Two wallets are enrolled and funded on the local chain".into();
        state.busy = false;
        state.chain_id = Some(contexts[0].chain_id.clone());
        state.token_id = Some(token_id);
        state.wallets = views;
        Ok(())
    }

    async fn execute(&self, command: Command) -> Result<()> {
        let mut guard = self.engine.lock().await;
        let engine = guard.as_mut().context("local environment is not ready")?;
        match command {
            Command::Sign(request, seed) => self.prepare(engine, request, seed).await,
            Command::Submit(request) => self.send(engine, request).await,
            Command::Refresh => {
                if let Some(result) = engine.chain.refresh_pending().await? {
                    self.record_result(engine, result).await
                } else {
                    self.refresh_balances(engine).await
                }
            }
        }
    }

    async fn refresh_balances(&self, engine: &Engine) -> Result<()> {
        let balances = engine.chain.balances().await?;
        let enrollments = [
            engine.chain.enrollment(0).await?,
            engine.chain.enrollment(1).await?,
        ];
        let mut state = self.state.lock().expect("demo state lock poisoned");
        for ((wallet, balance), enrollment) in
            state.wallets.iter_mut().zip(balances).zip(enrollments)
        {
            wallet.balance_base_units = balance;
            if let Some(enrollment) = enrollment {
                wallet.nonce = enrollment.next_nonce.to_string();
            }
        }
        state.busy = false;
        state.message = "Balances and nonces refreshed from the local contracts".into();
        Ok(())
    }

    async fn prepare(&self, engine: &mut Engine, request: SignRequest, seed: u64) -> Result<()> {
        ensure!(
            !engine.chain.has_pending(),
            "the previous transaction is unresolved"
        );
        engine.prepared = None;
        let amount = request.validate()?;
        let current = engine
            .chain
            .enrollment(request.wallet)
            .await?
            .context("wallet is not enrolled")?;
        let fingerprint = Arc::clone(&engine.wallets[request.wallet]);
        ensure!(
            current.active && current.commitment == fingerprint.commitment()?,
            "on-chain enrollment differs from this fingerprint's retained package"
        );
        let balances = engine.chain.balances().await?;
        ensure!(
            amount <= balances[request.wallet].parse::<u128>()?,
            "amount exceeds the wallet's current token balance"
        );
        let transfer = engine
            .chain
            .transfer_request(request.wallet, amount, request.memo)?;
        let expires_at_ms = engine
            .chain
            .block_time_ms()
            .await?
            .checked_add(300_000)
            .context("expiry overflow")?;
        let action = Action {
            operation: TRANSFER_OPERATION.into(),
            payload: transfer.signing_payload(),
            nonce: current.next_nonce,
            expires_at_ms,
        };

        engine.sequence = engine
            .sequence
            .checked_add(1)
            .context("capture sequence exhausted")?;
        let id = engine.sequence.to_string();
        self.progress("Selecting a new random circular sample from the fingerprint");
        let sampler = Arc::clone(&fingerprint);
        let mut sampled = tokio::task::spawn_blocking(move || sampler.sample(seed)).await??;
        let clean_view = fingerprint.view();
        let preview = CaptureView {
            id: id.clone(),
            wallet: request.wallet,
            image_url: format!("/api/capture/{id}"),
            width: clean_view.width,
            height: clean_view.height,
            region: sampled.metadata.circle,
            pattern_fraction: sampled.metadata.coverage_fraction,
            feature_count: None,
            points: Vec::new(),
            matched_feature_ids: Vec::new(),
        };
        *self.preview.lock().expect("capture preview lock poisoned") = Some(CapturePreview {
            id: id.clone(),
            png: std::mem::take(&mut sampled.png).into(),
        });
        self.state.lock().expect("demo state lock poisoned").capture = Some(preview);
        self.progress("Analyzing only the selected partial scan and searching for its signing key");
        let app = self.clone();
        let capture_id = id.clone();
        let signed = tokio::task::spawn_blocking(move || {
            fingerprint.sign(action, &sampled, |points| {
                let mut state = app.state.lock().expect("demo state lock poisoned");
                if let Some(capture) = state
                    .capture
                    .as_mut()
                    .filter(|capture| capture.id == capture_id)
                {
                    capture.feature_count = Some(points.len());
                    capture.points = points;
                }
            })
        })
        .await??;
        self.progress("Checking the signature and Merkle proof against the deployed wallet");
        ensure!(
            engine
                .chain
                .verify(request.wallet, &transfer, &signed.authorization)
                .await?,
            "the wallet rejected the reconstructed authorization"
        );
        let view = PreparedView {
            id: id.clone(),
            capture_id: id.clone(),
            wallet: request.wallet,
            recipient: request.recipient,
            amount: format_amount(amount),
            amount_base_units: amount.to_string(),
            memo: transfer.memo.clone(),
            expires_at_ms: expires_at_ms.to_string(),
            authorization: signed.authorization.clone(),
            matched_feature_ids: signed.matched_feature_ids,
            attempts: signed.attempts,
            verified: true,
        };
        engine.prepared = Some(Prepared {
            id,
            wallet: request.wallet,
            transfer,
            authorization: signed.authorization,
        });
        let mut state = self.state.lock().expect("demo state lock poisoned");
        for (wallet, balance) in state.wallets.iter_mut().zip(balances) {
            wallet.balance_base_units = balance;
        }
        state.wallets[request.wallet].nonce = current.next_nonce.to_string();
        if let Some(capture) = &mut state.capture {
            capture.matched_feature_ids = view.matched_feature_ids.clone();
        }
        state.prepared = Some(view);
        state.phase = "signed";
        state.message =
            "Signature and Merkle proof verified. Review and submit the approved transfer.".into();
        state.busy = false;
        Ok(())
    }

    async fn send(&self, engine: &mut Engine, request: SubmitRequest) -> Result<()> {
        let prepared = engine
            .prepared
            .take()
            .context("no unused signed transfer")?;
        ensure!(
            prepared.id == request.prepared_id,
            "signed transfer is stale"
        );
        ensure!(
            engine
                .chain
                .verify(prepared.wallet, &prepared.transfer, &prepared.authorization)
                .await?,
            "authorization expired or its nonce is no longer current; sign again"
        );
        let result = engine
            .chain
            .submit(prepared.wallet, &prepared.transfer, &prepared.authorization)
            .await?;
        self.record_result(engine, result).await
    }

    async fn record_result(&self, engine: &Engine, result: TransferResult) -> Result<()> {
        // Refresh each nonce from contract state. A view failure must never erase
        // the already observed transaction hash/result or claim a rollback.
        let mut nonces = [None, None];
        for (index, value) in nonces.iter_mut().enumerate() {
            if let Ok(Ok(Some(view))) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                engine.chain.enrollment(index),
            )
            .await
            {
                *value = Some(view.next_nonce.to_string());
            }
        }
        let mut state = self.state.lock().expect("demo state lock poisoned");
        if let Some(balances) = &result.balances {
            for (wallet, balance) in state.wallets.iter_mut().zip(balances) {
                wallet.balance_base_units = balance.clone();
            }
        }
        for (wallet, nonce) in state.wallets.iter_mut().zip(nonces) {
            if let Some(nonce) = nonce {
                wallet.nonce = nonce;
            }
        }
        let (phase, message) = match result.status {
            SubmissionStatus::Succeeded if result.balances.is_some() => (
                "complete",
                "NEAR confirmed the transfer; balances were read from the token contract",
            ),
            SubmissionStatus::Succeeded => (
                "complete",
                "NEAR confirmed the transfer; the balance refresh is unavailable",
            ),
            SubmissionStatus::Failed => (
                "failed",
                "The token transfer failed; the accepted nonce remains consumed",
            ),
            SubmissionStatus::Rejected => {
                ("failed", "The wallet rejected the transfer authorization")
            }
            SubmissionStatus::Pending | SubmissionStatus::Unknown => (
                "unknown",
                "The transaction outcome is unresolved. Check its status before signing again.",
            ),
        };
        state.phase = phase;
        state.message = message.into();
        state.transaction = Some(result);
        state.busy = false;
        Ok(())
    }
}

fn unresolved(result: &TransferResult) -> bool {
    matches!(
        result.status,
        SubmissionStatus::Pending | SubmissionStatus::Unknown
    )
}

#[cfg(test)]
mod tests;
