//! Per-session initialization and teardown; shared resources are owned by Demo.

use std::sync::{Arc, atomic::Ordering};

use anyhow::{Context, Result, ensure};

use super::{Engine, Session};
use crate::{
    model::{WalletView, hex},
    sessions::Startup,
};

impl Session {
    pub(super) async fn bootstrap(&self) -> Result<()> {
        let mut startup = self.startup.clone();
        let resources = loop {
            let current = startup.borrow_and_update().clone();
            match current {
                Startup::Ready(resources) => break resources,
                Startup::Preparing(message) => self.progress(&message),
                Startup::Failed(message) => anyhow::bail!("{message}"),
                Startup::Stopped => anyhow::bail!("demo is stopping"),
            }
            startup.changed().await.context("demo startup stopped")?;
        };
        ensure!(!self.expired(), "demo session has expired");
        self.progress("Creating your two wallets and token on the shared local NEAR chain");
        let mut guard = self.engine.lock().await;
        let (chain, contexts) = resources.sandbox.create_session().await?;
        *guard = Some(Engine {
            chain,
            wallets: Vec::new(),
            prepared: None,
            sequence: 0,
        });
        let engine = guard.as_mut().expect("session engine was just created");
        for (index, context) in contexts.iter().enumerate() {
            self.progress(&format!(
                "Generating your Merkle key set for Wallet {}",
                index + 1
            ));
            let context = context.clone();
            let template = Arc::clone(&resources.fingerprints[index]);
            let cancelled = Arc::clone(&self.cancelled);
            let permit = resources.work.clone().acquire_owned().await?;
            let wallet = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                ensure!(
                    !cancelled.load(Ordering::Acquire),
                    "demo session has expired"
                );
                template.enroll(&format!("near:{}", context.chain_id), &context.wallet_id)
            })
            .await??;
            ensure!(!self.expired(), "demo session has expired");
            engine.wallets.push(Arc::new(wallet));
        }
        self.progress("Registering your Merkle roots on your wallet contracts");
        let registrations = engine
            .chain
            .register([
                engine.wallets[0].commitment()?,
                engine.wallets[1].commitment()?,
            ])
            .await?;
        let balances = engine.chain.balances().await?;
        let views = (0..2)
            .map(|index| WalletView {
                id: index,
                name: format!("Wallet {}", index + 1),
                account_id: contexts[index].wallet_id.clone(),
                balance_base_units: balances[index].clone(),
                root: hex(&registrations[index].commitment.root),
                nonce: registrations[index].next_nonce.to_string(),
                leaf_count: registrations[index].commitment.leaf_count,
                fingerprint: engine.wallets[index].view(),
            })
            .collect();
        let mut state = self.state.lock().expect("demo state lock poisoned");
        state.phase = "ready";
        state.message = "Your two wallets are enrolled and funded on the local chain".into();
        state.busy = false;
        state.chain_id = Some(contexts[0].chain_id.clone());
        state.token_id = Some(engine.chain.token_id().to_owned());
        state.wallets = views;
        Ok(())
    }

    pub async fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        let task = self.task.lock().expect("demo task lock poisoned").take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        let engine = self.engine.lock().await.take();
        // Drop all biometric/session data before waiting for account cleanup.
        self.preview
            .lock()
            .expect("capture preview lock poisoned")
            .take();
        {
            let mut state = self.state.lock().expect("demo state lock poisoned");
            state.wallets.clear();
            state.prepared = None;
            state.capture = None;
            state.transaction = None;
        }
        if let Some(engine) = engine {
            let chain = engine.chain;
            drop(engine.wallets);
            drop(engine.prepared);
            // Account retirement revokes access and settles accepted transfers;
            // it never resends an uncertain transaction.
            if let Err(error) = chain.close().await {
                eprintln!("Session account cleanup failed: {error:#}");
            }
        }
    }
}
