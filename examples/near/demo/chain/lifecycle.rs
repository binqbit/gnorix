//! One sandbox process, with independently disposable account groups per visitor.

use std::sync::atomic::{AtomicBool, Ordering};

use gnorix_near_wallet::{TransferOutcome, TransferStatus};
use near_crypto::InMemorySigner;
use near_primitives::{types::BlockReference, views::QueryRequest};
use near_workspaces::types::{AccountId, KeyType, SecretKey};
use tokio::sync::{Mutex, oneshot};

use super::*;

const RETIREMENT_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy)]
struct SubmittedTransfer {
    hash: CryptoHash,
    wallet_index: usize,
    revision: u64,
    nonce: u64,
}

/// Shared runtime and immutable Wasm. Cloning this handle never starts a node.
#[derive(Clone)]
pub struct Sandbox {
    worker: Worker<SandboxNetwork>,
    chain_id: String,
    wallet_wasm: Arc<[u8]>,
    token_wasm: Arc<[u8]>,
    creation: Arc<Mutex<()>>,
}

impl Sandbox {
    pub async fn start(wasm_dir: &Path) -> Result<Self> {
        let wallet_wasm = read_wasm(wasm_dir, "gnorix_near_wallet.wasm")?.into();
        let token_wasm = read_wasm(wasm_dir, "gnorix_near_token.wasm")?.into();
        let worker = near_workspaces::sandbox_with_version("2.14.1").await?;
        let chain_id = worker.status().await?.chain_id;

        Ok(Self {
            worker,
            chain_id,
            wallet_wasm,
            token_wasm,
            creation: Arc::new(Mutex::new(())),
        })
    }

    /// Give a visitor two wallets, a standard token and an independent relayer.
    /// An owned initialization task finishes cleanup even if its HTTP/session
    /// caller disappears while account-creation receipts are still executing.
    pub async fn create_session(&self) -> Result<(Chain, [WalletContext; 2])> {
        let sandbox = self.clone();
        let (reply, response) = oneshot::channel();
        tokio::spawn(async move {
            // Parent creation spends the shared root access key. A cancelled
            // caller still in this queue does not need accounts created at all.
            let _creation = sandbox.creation.lock().await;
            if reply.is_closed() {
                return;
            }
            let result = sandbox.initialize_session().await;
            // If the caller disappeared during setup, dropping the undelivered
            // Chain schedules cleanup after all creation receipts have settled.
            let _ = reply.send(result);
        });
        response
            .await
            .context("session account initialization task failed")?
    }

    async fn initialize_session(&self) -> Result<(Chain, [WalletContext; 2])> {
        let accounts = Arc::new(SessionAccounts::new(&self.worker)?);
        self.initialize_accounts(accounts).await
    }

    async fn initialize_accounts(
        &self,
        accounts: Arc<SessionAccounts>,
    ) -> Result<(Chain, [WalletContext; 2])> {
        let mut cleanup = SetupCleanup(Some(Arc::clone(&accounts)));
        let result = self.build_session(Arc::clone(&accounts)).await;

        match result {
            Ok(session) => {
                cleanup.0 = None;
                Ok(session)
            }
            Err(error) => {
                if let Err(cleanup_error) = accounts.close().await {
                    return Err(error.context(format!(
                        "session initialization failed; cleanup also failed: {cleanup_error:#}"
                    )));
                }
                cleanup.0 = None;
                Err(error)
            }
        }
    }

    async fn build_session(
        &self,
        accounts: Arc<SessionAccounts>,
    ) -> Result<(Chain, [WalletContext; 2])> {
        accounts.create(&self.worker.root_account()?).await?;
        let wallets = [
            accounts.wallets[0]
                .deploy(&self.wallet_wasm)
                .await?
                .into_result()?,
            accounts.wallets[1]
                .deploy(&self.wallet_wasm)
                .await?
                .into_result()?,
        ];
        let token = accounts
            .token
            .deploy(&self.token_wasm)
            .await?
            .into_result()?;
        let relayer = accounts.relayer.clone();
        let signer = InMemorySigner::from_secret_key(
            relayer.id().clone(),
            relayer
                .secret_key()
                .to_string()
                .parse()
                .context("invalid generated relayer key")?,
        );
        let chain = Chain {
            worker: self.worker.clone(),
            accounts,
            rpc: JsonRpcClient::connect(self.worker.rpc_addr()),
            chain_id: self.chain_id.clone(),
            wallets,
            token,
            relayer,
            signer,
            pending: None,
        };

        chain.initialize_wallets().await?;
        chain.initialize_token().await?;
        ensure!(
            chain.balances().await? == [INITIAL_BALANCE.to_string(), INITIAL_BALANCE.to_string()],
            "initial token balances do not match confirmed chain state"
        );
        let contexts = chain.wallets.each_ref().map(|wallet| WalletContext {
            chain_id: chain.chain_id.clone(),
            wallet_id: wallet.id().to_string(),
        });
        Ok((chain, contexts))
    }
}

/// All account IDs and keys exist before the first transaction, so even a
/// deployment failure has a complete, bounded cleanup scope.
pub(super) struct SessionAccounts {
    parent: Account,
    wallets: [Account; 2],
    token: Account,
    relayer: Account,
    beneficiary: AccountId,
    rpc: JsonRpcClient,
    cleanup: Mutex<()>,
    closed: AtomicBool,
    enrollment_started: AtomicBool,
    last_submission: std::sync::Mutex<Option<SubmittedTransfer>>,
}

impl SessionAccounts {
    fn new(worker: &Worker<SandboxNetwork>) -> Result<Self> {
        let root = worker.root_account()?;
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).context("create session account namespace")?;
        let label: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let parent = account(worker, format!("s-{label}.{}", root.id()))?;

        Ok(Self {
            wallets: [
                account(worker, format!("wallet-1.{}", parent.id()))?,
                account(worker, format!("wallet-2.{}", parent.id()))?,
            ],
            token: account(worker, format!("token.{}", parent.id()))?,
            relayer: account(worker, format!("relayer.{}", parent.id()))?,
            parent,
            beneficiary: root.id().clone(),
            rpc: JsonRpcClient::connect(worker.rpc_addr()),
            cleanup: Mutex::new(()),
            closed: AtomicBool::new(false),
            enrollment_started: AtomicBool::new(false),
            last_submission: std::sync::Mutex::new(None),
        })
    }

    async fn create(&self, root: &Account) -> Result<()> {
        create_child(root, &self.parent, NearToken::from_near(100)).await?;
        for child in [
            &self.wallets[0],
            &self.wallets[1],
            &self.token,
            &self.relayer,
        ] {
            create_child(&self.parent, child, NearToken::from_near(20)).await?;
        }
        Ok(())
    }

    pub(super) async fn close(&self) -> Result<()> {
        timeout(RETIREMENT_TIMEOUT, self.retire())
            .await
            .context("session retirement timed out; unresolved accounts were retained")?
    }

    async fn retire(&self) -> Result<()> {
        let _cleanup = self.cleanup.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }

        // Partial setup cannot authorize transfers. Once registration starts,
        // even Drop must pass the same revocation and receipt-settlement gate.
        if self.enrollment_started.load(Ordering::Acquire) {
            self.revoke_enrollments().await?;
            self.settle_transfers().await?;
        }

        let mut failures = Vec::new();
        for account in self.all() {
            if let Err(error) = self.delete(account).await {
                failures.push(format!("{}: {error:#}", account.id()));
            }
        }
        ensure!(
            failures.is_empty(),
            "session account cleanup failed: {}",
            failures.join("; ")
        );
        self.closed.store(true, Ordering::Release);
        Ok(())
    }

    pub(super) fn mark_enrollment_started(&self) {
        self.enrollment_started.store(true, Ordering::Release);
    }

    pub(super) fn record_submission(
        &self,
        hash: CryptoHash,
        wallet_index: usize,
        revision: u64,
        nonce: u64,
    ) -> Result<()> {
        *self
            .last_submission
            .lock()
            .map_err(|_| anyhow::anyhow!("session transaction tracking is poisoned"))? =
            Some(SubmittedTransfer {
                hash,
                wallet_index,
                revision,
                nonce,
            });
        Ok(())
    }

    async fn revoke_enrollments(&self) -> Result<()> {
        for wallet in &self.wallets {
            if !self.exists(wallet.id()).await? {
                continue;
            }
            let enrollment: Option<EnrollmentView> =
                timeout(REQUEST_TIMEOUT, wallet.view(wallet.id(), "get_enrollment"))
                    .await
                    .context("enrollment retirement query timed out")??
                    .json()?;
            if enrollment.is_some_and(|view| view.active) {
                let outcome = timeout(
                    SUBMISSION_WAIT,
                    wallet
                        .call(wallet.id(), "revoke_enrollment")
                        .max_gas()
                        .transact(),
                )
                .await
                .context("enrollment revocation timed out")??;
                successful(outcome, "revoke expired session enrollment")?;
            }
            let confirmed: Option<EnrollmentView> =
                timeout(REQUEST_TIMEOUT, wallet.view(wallet.id(), "get_enrollment"))
                    .await
                    .context("enrollment revocation confirmation timed out")??
                    .json()?;
            ensure!(
                confirmed.is_none_or(|view| !view.active),
                "wallet {} still accepts authorizations; accounts retained",
                wallet.id()
            );
        }
        Ok(())
    }

    async fn settle_transfers(&self) -> Result<()> {
        let submitted = *self
            .last_submission
            .lock()
            .map_err(|_| anyhow::anyhow!("session transaction tracking is poisoned"))?;
        loop {
            let mut waiting = false;
            for (index, wallet) in self.wallets.iter().enumerate() {
                if !self.exists(wallet.id()).await? {
                    continue;
                }
                let outcome: TransferOutcome = timeout(
                    REQUEST_TIMEOUT,
                    wallet.view(wallet.id(), "get_transfer_status"),
                )
                .await
                .context("transfer retirement query timed out")??
                .json()?;
                if outcome.status == TransferStatus::Pending {
                    let finalized = match submitted.filter(|sent| {
                        sent.wallet_index == index
                            && sent.revision == outcome.revision.0
                            && sent.nonce == outcome.nonce.0
                    }) {
                        Some(sent) => self.submission_finalized(sent.hash).await?,
                        None => false,
                    };
                    waiting |= !finalized;
                }
            }
            if !waiting {
                return Ok(());
            }
            sleep(Duration::from_millis(250)).await;
        }
    }

    async fn submission_finalized(&self, hash: CryptoHash) -> Result<bool> {
        let response = timeout(
            REQUEST_TIMEOUT,
            self.rpc.call(methods::tx::RpcTransactionStatusRequest {
                transaction_info: methods::tx::TransactionInfo::TransactionId {
                    tx_hash: hash,
                    sender_account_id: self.relayer.id().clone(),
                },
                wait_until: TxExecutionStatus::Final,
            }),
        )
        .await;
        match response {
            Ok(Ok(response)) => Ok(response.final_execution_status == TxExecutionStatus::Final
                && response
                    .final_execution_outcome
                    .is_some_and(|outcome| outcome.into_outcome().transaction.hash == hash)),
            Ok(Err(JsonRpcError::ServerError(JsonRpcServerError::HandlerError(
                methods::tx::RpcTransactionError::UnknownTransaction { .. }
                | methods::tx::RpcTransactionError::TimeoutError(_),
            ))))
            | Err(_) => Ok(false),
            Ok(Err(error)) => Err(error.into()),
        }
    }

    fn all(&self) -> [&Account; 5] {
        [
            &self.wallets[0],
            &self.wallets[1],
            &self.token,
            &self.relayer,
            &self.parent,
        ]
    }

    async fn delete(&self, account: &Account) -> Result<()> {
        if !self.exists(account.id()).await? {
            return Ok(());
        }
        let outcome = timeout(
            REQUEST_TIMEOUT,
            account.clone().delete_account(&self.beneficiary),
        )
        .await
        .context("account deletion timed out")??;
        // A successful self-deletion can leave a failed gas-refund receipt
        // addressed to the deleted signer. Check the action's final result and
        // actual account absence, rather than rejecting that expected refund.
        outcome
            .into_result()
            .context("delete disposable session account")?;
        ensure!(
            !self.exists(account.id()).await?,
            "deleted session account {} is still present",
            account.id()
        );
        Ok(())
    }

    async fn exists(&self, id: &AccountId) -> Result<bool> {
        let request = methods::query::RpcQueryRequest {
            // A finalized block can predate the just-completed creation
            // receipt. Inspect the latest state before declaring it absent.
            block_reference: BlockReference::latest(),
            request: QueryRequest::ViewAccount {
                account_id: id.clone(),
            },
        };
        match timeout(REQUEST_TIMEOUT, self.rpc.call(request))
            .await
            .context("account cleanup query timed out")?
        {
            Ok(_) => Ok(true),
            Err(JsonRpcError::ServerError(JsonRpcServerError::HandlerError(
                methods::query::RpcQueryError::UnknownAccount { .. },
            ))) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn cleanup_later(self: &Arc<Self>) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let accounts = Arc::clone(self);
            runtime.spawn(async move {
                if let Err(error) = accounts.close().await {
                    eprintln!("Disposable session account cleanup failed: {error:#}");
                }
            });
        }
    }
}

struct SetupCleanup(Option<Arc<SessionAccounts>>);

impl Drop for SetupCleanup {
    fn drop(&mut self) {
        if let Some(accounts) = &self.0 {
            accounts.cleanup_later();
        }
    }
}

fn account(worker: &Worker<SandboxNetwork>, id: String) -> Result<Account> {
    Ok(Account::from_secret_key(
        id.parse()?,
        SecretKey::from_random(KeyType::ED25519),
        worker,
    ))
}

async fn create_child(parent: &Account, child: &Account, balance: NearToken) -> Result<()> {
    let suffix = format!(".{}", parent.id());
    let label = child
        .id()
        .as_str()
        .strip_suffix(&suffix)
        .context("invalid session account parent")?;
    parent
        .create_subaccount(label)
        .keys(child.secret_key().clone())
        .initial_balance(balance)
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "builds the NEAR contracts and verifies cleanup on an actual Sandbox"]
    async fn failed_contract_deployment_removes_every_preallocated_account() -> Result<()> {
        let wasm = crate::runtime::prepare_contracts(|_| {}).await?;
        let mut sandbox = Sandbox::start(&wasm).await?;
        // Both wallet deployments and all account creations precede this error.
        sandbox.token_wasm = Arc::from(b"invalid Wasm".as_slice());
        let accounts = Arc::new(SessionAccounts::new(&sandbox.worker)?);
        assert!(
            sandbox
                .initialize_accounts(Arc::clone(&accounts))
                .await
                .is_err()
        );
        for account in accounts.all() {
            assert!(
                !accounts.exists(account.id()).await?,
                "left {} behind",
                account.id()
            );
        }
        accounts.close().await?;
        sandbox.worker.status().await?;
        Ok(())
    }
}
