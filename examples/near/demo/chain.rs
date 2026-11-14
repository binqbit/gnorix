//! Managed local chain and narrowly scoped, observable token transfers.
//!
//! Account keys stay inside this module. A transfer is signed once and broadcast
//! once; uncertainty retains its transaction hash and never triggers a resend.

mod transaction;

#[cfg(test)]
mod tests;

use transaction::{PendingTransfer, summarize_execution};
pub use transaction::{SubmissionStatus, TransferResult};

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use gnorix::near::{Authorization, Commitment, EnrollmentView};
use gnorix_near_wallet::Transfer;
use near_crypto::{InMemorySigner, Signer};
use near_jsonrpc_client::{
    JsonRpcClient,
    errors::{JsonRpcError, JsonRpcServerError},
    methods,
};
use near_primitives::{
    hash::CryptoHash,
    transaction::{
        Action as TransactionAction, FunctionCallAction, SignedTransaction, Transaction,
        TransactionV0,
    },
    views::{FinalExecutionOutcomeView, TxExecutionStatus},
};
use near_sdk::serde::Serialize;
use near_workspaces::{
    Account, Contract, Worker, network::Sandbox, result::ExecutionFinalResult, types::NearToken,
};
use serde_json::json;
use tokio::time::{Instant, sleep, timeout};

const TOKEN_UNIT: u128 = 10_u128.pow(24);
const INITIAL_BALANCE: u128 = 1_000 * TOKEN_UNIT;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const SUBMISSION_WAIT: Duration = Duration::from_secs(20);
/// Attached to each demo transfer, distinct from the gas its receipts consume.
pub const TRANSFER_GAS_TGAS: u64 = 100;

#[derive(Clone, Debug, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct WalletContext {
    pub chain_id: String,
    pub wallet_id: String,
}

pub struct Chain {
    worker: Worker<Sandbox>,
    rpc: JsonRpcClient,
    chain_id: String,
    wallets: [Contract; 2],
    token: Contract,
    relayer: Account,
    signer: Signer,
    pending: Option<PendingTransfer>,
}

impl Chain {
    /// Creates a fresh, disposable chain. Both initial balances are checked via
    /// the token's RPC views after minting and an ordinary standard transfer.
    pub async fn bootstrap(wasm_dir: &Path) -> Result<(Self, [WalletContext; 2])> {
        let wallet_wasm = read_wasm(wasm_dir, "gnorix_near_wallet.wasm")?;
        let token_wasm = read_wasm(wasm_dir, "gnorix_near_token.wasm")?;
        let worker = near_workspaces::sandbox_with_version("2.14.1").await?;
        let wallets = [
            worker.dev_deploy(&wallet_wasm).await?,
            worker.dev_deploy(&wallet_wasm).await?,
        ];
        let token = worker.dev_deploy(&token_wasm).await?;
        let relayer = worker.dev_create_account().await?;
        let signer = InMemorySigner::from_secret_key(
            relayer.id().clone(),
            relayer
                .secret_key()
                .to_string()
                .parse()
                .context("invalid generated relayer key")?,
        );
        let chain_id = worker.status().await?.chain_id;
        let rpc = JsonRpcClient::connect(worker.rpc_addr());
        let chain = Self {
            worker,
            rpc,
            chain_id,
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

    pub fn token_id(&self) -> &str {
        self.token.id().as_str()
    }

    pub fn relayer_id(&self) -> &str {
        self.relayer.id().as_str()
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Prevalidate both domains before sending either registration. These are
    /// separate transactions; startup must stop if either registration fails.
    pub async fn register(&self, commitments: [Commitment; 2]) -> Result<[EnrollmentView; 2]> {
        for (index, commitment) in commitments.iter().enumerate() {
            commitment.validate()?;
            let wallet = &self.wallets[index];
            ensure!(
                commitment.network == format!("near:{}", self.chain_id)
                    && commitment.contract_id.as_str() == wallet.id().as_str()
                    && commitment.owner_id.as_str() == wallet.id().as_str()
                    && commitment.revision == 1,
                "commitment does not belong to its demo wallet"
            );
            ensure!(
                self.enrollment(index).await?.is_none(),
                "demo wallet is already enrolled"
            );
        }
        for (wallet, commitment) in self.wallets.iter().zip(&commitments) {
            successful(
                wallet
                    .call("register_enrollment")
                    .args_json(json!({"commitment": commitment}))
                    .deposit(NearToken::from_millinear(100))
                    .max_gas()
                    .transact()
                    .await?,
                "register wallet commitment",
            )?;
        }
        let registered = [
            self.enrollment(0)
                .await?
                .context("wallet 1 enrollment missing")?,
            self.enrollment(1)
                .await?
                .context("wallet 2 enrollment missing")?,
        ];
        for (view, commitment) in registered.iter().zip(commitments) {
            ensure!(
                view.active && view.next_nonce == 0 && view.commitment == commitment,
                "registered commitment differs from the requested enrollment"
            );
        }
        Ok(registered)
    }

    pub async fn enrollment(&self, wallet_index: usize) -> Result<Option<EnrollmentView>> {
        let view = timeout(
            REQUEST_TIMEOUT,
            self.wallet(wallet_index)?.view("get_enrollment"),
        )
        .await
        .context("enrollment query timed out")??;
        Ok(view.json()?)
    }

    pub async fn balances(&self) -> Result<[String; 2]> {
        Ok([self.balance(0).await?, self.balance(1).await?])
    }

    pub async fn block_time_ms(&self) -> Result<u64> {
        let block = timeout(REQUEST_TIMEOUT, self.worker.view_block())
            .await
            .context("block time query timed out")??;
        Ok(block.timestamp() / 1_000_000)
    }

    pub fn transfer_request(
        &self,
        wallet_index: usize,
        amount: u128,
        memo: Option<String>,
    ) -> Result<Transfer> {
        self.wallet(wallet_index)?;
        ensure!(amount > 0, "transfer amount must be positive");
        ensure!(
            memo.as_ref().is_none_or(|text| text.len() <= 256),
            "memo exceeds 256 UTF-8 bytes"
        );
        Ok(Transfer {
            token_id: self.token.id().clone(),
            receiver_id: self.wallets[1 - wallet_index].id().clone(),
            amount: amount.into(),
            memo,
        })
    }

    pub async fn verify(
        &self,
        wallet_index: usize,
        transfer: &Transfer,
        authorization: &Authorization,
    ) -> Result<bool> {
        self.validate_transfer(wallet_index, transfer)?;
        let query = self
            .wallet(wallet_index)?
            .view("verify_transfer")
            .args_json(json!({"transfer": transfer, "authorization": authorization}));
        let view = timeout(REQUEST_TIMEOUT, query)
            .await
            .context("signature verification query timed out")??;
        Ok(view.json()?)
    }

    /// Persist the exact transaction hash before broadcasting. Once this method
    /// starts broadcasting, errors become observations rather than retry advice.
    pub async fn submit(
        &mut self,
        wallet_index: usize,
        transfer: &Transfer,
        authorization: &Authorization,
    ) -> Result<TransferResult> {
        ensure!(
            !self.has_pending(),
            "a previous transaction is unresolved; check its status first"
        );
        self.validate_transfer(wallet_index, transfer)?;
        let transaction = self
            .sign_transaction(wallet_index, transfer, authorization)
            .await?;
        let hash = transaction.get_hash();
        self.pending = Some(PendingTransfer {
            hash,
            result: TransferResult {
                wallet_index,
                revision: authorization.revision.to_string(),
                nonce: authorization.action.nonce.to_string(),
                tx_hash: Some(hash.to_string()),
                relayer_id: self.relayer_id().to_owned(),
                status: SubmissionStatus::Pending,
                callback: None,
                gas_burnt: None,
                failures: vec![],
                logs: vec![],
                balances: None,
                observation_error: None,
            },
        });

        let response = timeout(
            REQUEST_TIMEOUT,
            self.rpc.call(methods::send_tx::RpcSendTransactionRequest {
                signed_transaction: transaction,
                wait_until: TxExecutionStatus::Included,
            }),
        )
        .await;
        match response {
            Ok(Ok(_)) => self.wait_for_execution().await,
            Ok(Err(JsonRpcError::ServerError(JsonRpcServerError::HandlerError(
                methods::tx::RpcTransactionError::InvalidTransaction { context },
            )))) => {
                let mut result = self
                    .pending
                    .take()
                    .expect("pending transaction was just recorded")
                    .result;
                result.status = SubmissionStatus::Rejected;
                result.observation_error =
                    Some(format!("transaction rejected before execution: {context}"));
                self.observe_balances(&mut result).await;
                Ok(result)
            }
            other => {
                let reason = match other {
                    Ok(Err(error)) => format!("broadcast acknowledgement unavailable: {error}"),
                    Err(_) => {
                        "broadcast acknowledgement timed out; transaction may have been accepted"
                            .to_owned()
                    }
                    Ok(Ok(_)) => unreachable!("successful broadcast handled above"),
                };
                self.pending_observation(SubmissionStatus::Unknown, Some(reason))
                    .await
            }
        }
    }

    /// Only queries the retained hash. It never broadcasts a transaction.
    pub async fn refresh_pending(&mut self) -> Result<Option<TransferResult>> {
        if self.pending.is_none() {
            return Ok(None);
        }
        let observation = self.observe_transaction().await;
        Ok(Some(self.apply_observation(observation).await?))
    }

    fn wallet(&self, index: usize) -> Result<&Contract> {
        self.wallets
            .get(index)
            .context("wallet index must be 0 or 1")
    }

    fn validate_transfer(&self, index: usize, transfer: &Transfer) -> Result<()> {
        let expected = self.transfer_request(index, transfer.amount.0, transfer.memo.clone())?;
        ensure!(
            *transfer == expected,
            "only transfers between the two demo wallets using the demo token are allowed"
        );
        Ok(())
    }

    async fn initialize_wallets(&self) -> Result<()> {
        for wallet in &self.wallets {
            successful(
                wallet
                    .call("new")
                    .args_json(json!({"owner_id": wallet.id()}))
                    .transact()
                    .await?,
                "initialize wallet",
            )?;
        }
        Ok(())
    }

    async fn initialize_token(&self) -> Result<()> {
        let supply = (INITIAL_BALANCE * 2).to_string();
        successful(
            self.token
                .call("new")
                .args_json(json!({"owner_id": self.wallets[0].id(), "total_supply": supply}))
                .transact()
                .await?,
            "initialize token supply",
        )?;
        successful(
            self.wallets[0]
                .as_account()
                .call(self.token.id(), "storage_deposit")
                .args_json(json!({"account_id": self.wallets[1].id(), "registration_only": true}))
                .deposit(NearToken::from_millinear(100))
                .max_gas()
                .transact()
                .await?,
            "register second wallet with token",
        )?;
        let funding = json!({
            "receiver_id": self.wallets[1].id(),
            "amount": INITIAL_BALANCE.to_string(),
            "memo": "initial demo funding",
        });
        successful(
            self.wallets[0]
                .as_account()
                .call(self.token.id(), "ft_transfer")
                .args_json(funding)
                .deposit(NearToken::from_yoctonear(1))
                .max_gas()
                .transact()
                .await?,
            "fund second wallet",
        )?;
        Ok(())
    }

    async fn balance(&self, index: usize) -> Result<String> {
        let query = self
            .token
            .view("ft_balance_of")
            .args_json(json!({"account_id": self.wallet(index)?.id()}));
        let view = timeout(REQUEST_TIMEOUT, query)
            .await
            .context("token balance query timed out")??;
        let value: String = view.json()?;
        let amount: u128 = value
            .parse()
            .context("token returned a non-integer balance")?;
        ensure!(
            amount.to_string() == value,
            "token returned a noncanonical balance"
        );
        Ok(value)
    }

    async fn sign_transaction(
        &self,
        index: usize,
        transfer: &Transfer,
        authorization: &Authorization,
    ) -> Result<SignedTransaction> {
        let access = timeout(
            REQUEST_TIMEOUT,
            self.relayer
                .view_access_key(&self.relayer.secret_key().public_key()),
        )
        .await
        .context("relayer access key query timed out")??;
        let nonce = access
            .nonce
            .checked_add(1)
            .context("relayer nonce exhausted")?;
        let block = timeout(REQUEST_TIMEOUT, self.worker.view_block())
            .await
            .context("transaction block query timed out")??;
        let transaction = Transaction::V0(TransactionV0 {
            signer_id: self.relayer.id().clone(),
            public_key: self.signer.public_key(),
            nonce,
            receiver_id: self.wallet(index)?.id().clone(),
            block_hash: CryptoHash(block.hash().0),
            actions: vec![TransactionAction::FunctionCall(Box::new(
                FunctionCallAction {
                    method_name: "transfer".into(),
                    args: serde_json::to_vec(
                        &json!({"transfer": transfer, "authorization": authorization}),
                    )?,
                    gas: near_primitives::gas::Gas::from_teragas(TRANSFER_GAS_TGAS),
                    deposit: near_primitives::types::Balance::ZERO,
                },
            ))],
        });
        let (hash, _) = transaction.get_hash_and_size();
        Ok(SignedTransaction::new(
            self.signer.sign(hash.as_ref()),
            transaction,
        ))
    }

    async fn wait_for_execution(&mut self) -> Result<TransferResult> {
        let deadline = Instant::now() + SUBMISSION_WAIT;
        loop {
            let observation = self.observe_transaction().await;
            let keep_waiting =
                matches!(&observation, Observation::Pending) && Instant::now() < deadline;
            if !keep_waiting {
                return self.apply_observation(observation).await;
            }
            sleep(Duration::from_millis(250)).await;
        }
    }

    async fn observe_transaction(&self) -> Observation {
        let pending = self
            .pending
            .as_ref()
            .expect("observation requires a pending transaction");
        let response = timeout(
            REQUEST_TIMEOUT,
            self.rpc.call(methods::tx::RpcTransactionStatusRequest {
                transaction_info: methods::tx::TransactionInfo::TransactionId {
                    tx_hash: pending.hash,
                    sender_account_id: self.relayer.id().clone(),
                },
                wait_until: TxExecutionStatus::Final,
            }),
        )
        .await;
        match response {
            Ok(Ok(response)) => {
                if response.final_execution_status != TxExecutionStatus::Final {
                    return Observation::Pending;
                }
                match response.final_execution_outcome {
                    Some(outcome) => Observation::Finished(Box::new(outcome.into_outcome())),
                    None => Observation::Pending,
                }
            }
            Ok(Err(JsonRpcError::ServerError(JsonRpcServerError::HandlerError(
                methods::tx::RpcTransactionError::UnknownTransaction { .. }
                | methods::tx::RpcTransactionError::TimeoutError(_),
            )))) => Observation::Pending,
            Ok(Err(error)) => {
                Observation::Unknown(format!("transaction status unavailable: {error}"))
            }
            Err(_) => Observation::Unknown("transaction status request timed out".into()),
        }
    }

    async fn apply_observation(&mut self, observation: Observation) -> Result<TransferResult> {
        match observation {
            Observation::Pending => {
                self.pending_observation(SubmissionStatus::Pending, None)
                    .await
            }
            Observation::Unknown(error) => {
                self.pending_observation(SubmissionStatus::Unknown, Some(error))
                    .await
            }
            Observation::Finished(outcome) => {
                let pending = self
                    .pending
                    .as_ref()
                    .context("pending transaction missing")?;
                let mut result = summarize_execution(&outcome, pending);
                self.observe_balances(&mut result).await;
                if matches!(
                    result.status,
                    SubmissionStatus::Succeeded
                        | SubmissionStatus::Failed
                        | SubmissionStatus::Rejected
                ) {
                    self.pending = None;
                } else if let Some(pending) = &mut self.pending {
                    pending.result = result.clone();
                }
                Ok(result)
            }
        }
    }

    async fn pending_observation(
        &mut self,
        status: SubmissionStatus,
        error: Option<String>,
    ) -> Result<TransferResult> {
        let mut result = self
            .pending
            .as_ref()
            .context("pending transaction missing")?
            .result
            .clone();
        result.status = status;
        result.observation_error = error;
        self.observe_balances(&mut result).await;
        self.pending
            .as_mut()
            .expect("pending observation does not clear the transaction")
            .result = result.clone();
        Ok(result)
    }

    async fn observe_balances(&self, result: &mut TransferResult) {
        match timeout(REQUEST_TIMEOUT, self.balances()).await {
            Ok(Ok(balances)) => result.balances = Some(balances),
            error => {
                result.balances = None;
                let reason = match error {
                    Ok(Err(error)) => format!("balance refresh failed: {error}"),
                    _ => "balance refresh timed out".to_owned(),
                };
                result.observation_error = Some(match result.observation_error.take() {
                    Some(previous) => format!("{previous}; {reason}"),
                    None => reason,
                });
            }
        }
    }
}

enum Observation {
    Pending,
    Unknown(String),
    Finished(Box<FinalExecutionOutcomeView>),
}

fn read_wasm(directory: &Path, name: &str) -> Result<Vec<u8>> {
    let path = directory.join(name);
    std::fs::read(&path).with_context(|| format!("cannot read contract Wasm {}", path.display()))
}

fn successful(result: ExecutionFinalResult, operation: &str) -> Result<()> {
    if let Some(failure) = result.failures().first() {
        let error = (*failure)
            .clone()
            .into_result()
            .err()
            .context("failure outcome without an error")?;
        bail!("{operation}: {error}");
    }
    result.into_result().with_context(|| operation.to_owned())?;
    Ok(())
}
