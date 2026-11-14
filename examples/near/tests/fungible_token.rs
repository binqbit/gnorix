//! Standard fungible-token behavior in a real local NEAR runtime.
//!
//! These checks deliberately use ordinary accounts and NEP interfaces. Gnorix
//! authorization belongs to a separate wallet, not to the token contract.

use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use near_workspaces::{
    Account, Contract, Worker, network::Sandbox, result::ExecutionFinalResult, types::NearToken,
};
use serde_json::{Value, json};

const TOKEN_UNIT: u128 = 10_u128.pow(24);
const TOTAL_SUPPLY: u128 = 1_000_000 * TOKEN_UNIT;

struct Scenario {
    token: Contract,
    receiver: Contract,
    owner: Account,
    alice: Account,
    bob: Account,
    _worker: Worker<Sandbox>,
}

impl Scenario {
    async fn start() -> Result<(Self, ExecutionFinalResult)> {
        let worker = near_workspaces::sandbox_with_version("2.14.1").await?;
        let token = worker.dev_deploy(&wasm("gnorix_near_token.wasm")?).await?;
        let receiver = worker
            .dev_deploy(&wasm("gnorix_ft_test_receiver.wasm")?)
            .await?;
        let owner = worker.dev_create_account().await?;
        let alice = worker.dev_create_account().await?;
        let bob = worker.dev_create_account().await?;

        rejected(
            owner
                .call(token.id(), "new")
                .args_json(
                    json!({"owner_id": owner.id(), "total_supply": TOTAL_SUPPLY.to_string()}),
                )
                .transact()
                .await?,
            "initialization by another account",
        )?;
        let mint = token
            .call("new")
            .args_json(json!({"owner_id": owner.id(), "total_supply": TOTAL_SUPPLY.to_string()}))
            .transact()
            .await?;
        successful(&mint, "token initialization")?;
        receiver
            .call("new")
            .args_json(json!({"token_id": token.id()}))
            .transact()
            .await?
            .into_result()?;

        Ok((
            Self {
                token,
                receiver,
                owner,
                alice,
                bob,
                _worker: worker,
            },
            mint,
        ))
    }

    async fn balance(&self, account_id: &str) -> Result<u128> {
        let value: String = self
            .token
            .view("ft_balance_of")
            .args_json(json!({"account_id": account_id}))
            .await?
            .json()?;
        Ok(value.parse()?)
    }

    async fn supply(&self) -> Result<u128> {
        let value: String = self.token.view("ft_total_supply").await?.json()?;
        Ok(value.parse()?)
    }

    async fn storage(&self, account_id: &str) -> Result<Value> {
        Ok(self
            .token
            .view("storage_balance_of")
            .args_json(json!({"account_id": account_id}))
            .await?
            .json()?)
    }

    async fn call(
        &self,
        caller: &Account,
        method: &str,
        args: Value,
        deposit: u128,
    ) -> Result<ExecutionFinalResult> {
        Ok(caller
            .call(self.token.id(), method)
            .args_json(args)
            .deposit(NearToken::from_yoctonear(deposit))
            .max_gas()
            .transact()
            .await?)
    }

    async fn transfer(
        &self,
        caller: &Account,
        receiver_id: &str,
        amount: u128,
        deposit: u128,
    ) -> Result<ExecutionFinalResult> {
        self.call(
            caller,
            "ft_transfer",
            json!({
                "receiver_id": receiver_id,
                "amount": amount.to_string(),
                "memo": "standard transfer",
            }),
            deposit,
        )
        .await
    }
}

fn wasm(name: &str) -> Result<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/near")
        .join(name);
    std::fs::read(&path)
        .with_context(|| format!("build the NEAR example first: {}", path.display()))
}

fn successful(result: &ExecutionFinalResult, context: &str) -> Result<()> {
    ensure!(
        result.is_success(),
        "{context}: final execution failed: {result:?}"
    );
    ensure!(
        result.receipt_failures().is_empty(),
        "{context}: a receipt failed: {result:?}"
    );
    Ok(())
}

fn rejected(result: ExecutionFinalResult, context: &str) -> Result<()> {
    ensure!(
        result.is_failure(),
        "{context}: transaction unexpectedly succeeded"
    );
    Ok(())
}

fn events(result: &ExecutionFinalResult) -> Result<Vec<Value>> {
    result
        .logs()
        .into_iter()
        .filter_map(|line| line.strip_prefix("EVENT_JSON:"))
        .map(|event| serde_json::from_str(event).map_err(Into::into))
        .collect()
}

fn expected_event(event: &str, data: Value) -> Value {
    json!({"standard": "nep141", "version": "1.0.0", "event": event, "data": [data]})
}

async fn fixed_supply_and_metadata(scenario: &Scenario, mint: &ExecutionFinalResult) -> Result<()> {
    let metadata: Value = scenario.token.view("ft_metadata").await?.json()?;
    ensure!(metadata["spec"] == "ft-1.0.0");
    ensure!(metadata["name"] == "Gnorix Test Token");
    ensure!(metadata["symbol"] == "GXT");
    ensure!(metadata["decimals"] == 24);
    ensure!(scenario.supply().await? == TOTAL_SUPPLY);
    ensure!(scenario.balance(scenario.owner.id().as_str()).await? == TOTAL_SUPPLY);
    ensure!(scenario.balance(scenario.alice.id().as_str()).await? == 0);
    ensure!(
        events(mint)?
            == vec![expected_event(
                "ft_mint",
                json!({
                    "owner_id": scenario.owner.id(), "amount": TOTAL_SUPPLY.to_string(), "memo": "initial supply",
                })
            )]
    );
    rejected(
        scenario
            .token
            .call("new")
            .args_json(json!({"owner_id": scenario.owner.id(), "total_supply": "1"}))
            .transact()
            .await?,
        "reinitialization",
    )
    .map(|_| ())
}

async fn storage_registration_and_refunds(scenario: &Scenario) -> Result<u128> {
    let bounds: Value = scenario
        .token
        .view("storage_balance_bounds")
        .await?
        .json()?;
    let minimum: u128 = bounds["min"]
        .as_str()
        .context("storage minimum must be a decimal string")?
        .parse()?;
    ensure!(minimum > 0 && bounds["min"] == bounds["max"]);
    let expected = json!({"total": minimum.to_string(), "available": "0"});
    ensure!(scenario.storage(scenario.owner.id().as_str()).await? == expected);
    ensure!(
        scenario
            .storage(scenario.alice.id().as_str())
            .await?
            .is_null()
    );

    rejected(
        scenario
            .call(
                &scenario.owner,
                "storage_deposit",
                json!({
                    "account_id": scenario.alice.id(), "registration_only": true,
                }),
                minimum - 1,
            )
            .await?,
        "insufficient storage payment",
    )?;
    ensure!(
        scenario
            .storage(scenario.alice.id().as_str())
            .await?
            .is_null()
    );

    let deposit = NearToken::from_near(1).as_yoctonear();
    ensure!(deposit > minimum);
    let before = scenario.owner.view_account().await?.balance.as_yoctonear();
    let registered = scenario
        .call(
            &scenario.owner,
            "storage_deposit",
            json!({
                "account_id": scenario.alice.id(), "registration_only": true,
            }),
            deposit,
        )
        .await?;
    successful(&registered, "storage registration and excess refund")?;
    ensure!(registered.json::<Value>()? == expected);
    let after = scenario.owner.view_account().await?.balance.as_yoctonear();
    let spent = before
        .checked_sub(after)
        .context("registration unexpectedly credited its payer")?;
    // The generous gas allowance distinguishes the refund from retaining 1 NEAR.
    let gas_allowance = NearToken::from_millinear(100).as_yoctonear();
    ensure!(
        spent >= minimum && spent < minimum + gas_allowance,
        "excess storage deposit was not refunded"
    );

    let before = scenario.owner.view_account().await?.balance.as_yoctonear();
    let duplicate = scenario
        .call(
            &scenario.owner,
            "storage_deposit",
            json!({
                "account_id": scenario.alice.id(), "registration_only": false,
            }),
            deposit,
        )
        .await?;
    successful(&duplicate, "duplicate registration refund")?;
    ensure!(duplicate.json::<Value>()? == expected);
    let after = scenario.owner.view_account().await?.balance.as_yoctonear();
    ensure!(
        before.saturating_sub(after) < gas_allowance,
        "duplicate registration retained a deposit"
    );

    for account_id in [scenario.receiver.id(), scenario.bob.id()] {
        let result = scenario
            .call(
                &scenario.owner,
                "storage_deposit",
                json!({
                    "account_id": account_id, "registration_only": true,
                }),
                minimum,
            )
            .await?;
        successful(&result, "storage registration")?;
        ensure!(scenario.storage(account_id.as_str()).await? == expected);
    }

    rejected(
        scenario
            .call(
                &scenario.alice,
                "storage_withdraw",
                json!({"amount": "0"}),
                0,
            )
            .await?,
        "withdraw without one yocto",
    )?;
    rejected(
        scenario
            .call(
                &scenario.alice,
                "storage_withdraw",
                json!({"amount": "1"}),
                1,
            )
            .await?,
        "withdraw unavailable storage",
    )?;
    let withdrawal = scenario
        .call(
            &scenario.alice,
            "storage_withdraw",
            json!({"amount": "0"}),
            1,
        )
        .await?;
    successful(&withdrawal, "zero available storage withdrawal")?;
    ensure!(withdrawal.json::<Value>()? == expected);
    Ok(minimum)
}

async fn ordinary_transfers(scenario: &Scenario) -> Result<()> {
    let amount = 500 * TOKEN_UNIT;
    for deposit in [0, 2] {
        rejected(
            scenario
                .transfer(
                    &scenario.owner,
                    scenario.alice.id().as_str(),
                    amount,
                    deposit,
                )
                .await?,
            "transfer requires exactly one yocto",
        )?;
    }
    rejected(
        scenario
            .transfer(&scenario.bob, scenario.alice.id().as_str(), amount, 1)
            .await?,
        "another account cannot spend the owner's tokens",
    )?;
    rejected(
        scenario
            .transfer(&scenario.owner, scenario.owner.id().as_str(), amount, 1)
            .await?,
        "self transfer",
    )?;
    rejected(
        scenario
            .transfer(&scenario.owner, scenario.alice.id().as_str(), 0, 1)
            .await?,
        "zero transfer",
    )?;
    rejected(
        scenario
            .transfer(
                &scenario.owner,
                scenario.alice.id().as_str(),
                TOTAL_SUPPLY + 1,
                1,
            )
            .await?,
        "insufficient token balance",
    )?;
    ensure!(scenario.balance(scenario.owner.id().as_str()).await? == TOTAL_SUPPLY);
    ensure!(scenario.balance(scenario.alice.id().as_str()).await? == 0);

    let transfer = scenario
        .transfer(&scenario.owner, scenario.alice.id().as_str(), amount, 1)
        .await?;
    successful(&transfer, "ordinary owner transfer")?;
    ensure!(
        events(&transfer)?
            == vec![expected_event(
                "ft_transfer",
                json!({
                    "old_owner_id": scenario.owner.id(), "new_owner_id": scenario.alice.id(),
                    "amount": amount.to_string(), "memo": "standard transfer",
                })
            )]
    );
    let returned = scenario
        .transfer(
            &scenario.alice,
            scenario.owner.id().as_str(),
            10 * TOKEN_UNIT,
            1,
        )
        .await?;
    successful(&returned, "ordinary recipient transfer")?;
    ensure!(scenario.balance(scenario.alice.id().as_str()).await? == 490 * TOKEN_UNIT);
    ensure!(
        scenario.balance(scenario.owner.id().as_str()).await? == TOTAL_SUPPLY - 490 * TOKEN_UNIT
    );
    ensure!(scenario.supply().await? == TOTAL_SUPPLY);

    rejected(scenario.call(&scenario.owner, "ft_resolve_transfer", json!({
        "sender_id": scenario.owner.id(), "receiver_id": scenario.alice.id(), "amount": "1",
    }), 0).await?, "external caller cannot invoke the private resolver")
}

async fn transfer_call_refunds(scenario: &Scenario) -> Result<()> {
    for (mode, tokens, retained) in [
        ("keep", 20, 20),
        ("refund_half", 30, 15),
        ("refund_all", 40, 0),
        ("reject", 50, 0),
    ] {
        let amount = tokens * TOKEN_UNIT;
        let used = retained * TOKEN_UNIT;
        let owner_before = scenario.balance(scenario.owner.id().as_str()).await?;
        let receiver_before = scenario.balance(scenario.receiver.id().as_str()).await?;
        let result = scenario.call(&scenario.owner, "ft_transfer_call", json!({
            "receiver_id": scenario.receiver.id(), "amount": amount.to_string(), "memo": mode, "msg": mode,
        }), 1).await?;

        // A rejected receiver receipt is resolved successfully by refunding tokens.
        ensure!(result.is_success(), "{mode}: resolver did not complete");
        ensure!(result.receipt_failures().is_empty() == (mode != "reject"));
        let mut expected = vec![expected_event(
            "ft_transfer",
            json!({
                "old_owner_id": scenario.owner.id(), "new_owner_id": scenario.receiver.id(),
                "amount": amount.to_string(), "memo": mode,
            }),
        )];
        if used < amount {
            expected.push(expected_event(
                "ft_transfer",
                json!({
                    "old_owner_id": scenario.receiver.id(), "new_owner_id": scenario.owner.id(),
                    "amount": (amount - used).to_string(), "memo": "refund",
                }),
            ));
        }
        ensure!(
            events(&result)? == expected,
            "{mode}: wrong transfer/refund events"
        );
        ensure!(
            result.json::<String>()? == used.to_string(),
            "{mode}: wrong used amount"
        );
        ensure!(scenario.balance(scenario.owner.id().as_str()).await? == owner_before - used);
        ensure!(scenario.balance(scenario.receiver.id().as_str()).await? == receiver_before + used);
        ensure!(scenario.supply().await? == TOTAL_SUPPLY);
    }
    Ok(())
}

async fn unregister_and_burn(scenario: &Scenario, minimum: u128) -> Result<()> {
    rejected(
        scenario
            .call(
                &scenario.alice,
                "storage_unregister",
                json!({"force": true}),
                0,
            )
            .await?,
        "unregister without one yocto",
    )?;
    rejected(
        scenario
            .call(
                &scenario.alice,
                "storage_unregister",
                json!({"force": false}),
                1,
            )
            .await?,
        "unregister positive token balance without force",
    )?;
    ensure!(scenario.balance(scenario.alice.id().as_str()).await? == 490 * TOKEN_UNIT);
    ensure!(
        !scenario
            .storage(scenario.alice.id().as_str())
            .await?
            .is_null()
    );

    let empty = scenario
        .call(
            &scenario.bob,
            "storage_unregister",
            json!({"force": false}),
            1,
        )
        .await?;
    successful(&empty, "empty-account storage refund")?;
    ensure!(events(&empty)?.is_empty());
    ensure!(empty.json::<bool>()?);
    ensure!(
        scenario
            .storage(scenario.bob.id().as_str())
            .await?
            .is_null()
    );
    let missing = scenario
        .call(
            &scenario.bob,
            "storage_unregister",
            json!({"force": false}),
            1,
        )
        .await?;
    successful(&missing, "already unregistered account")?;
    ensure!(!missing.json::<bool>()?);
    rejected(
        scenario
            .transfer(&scenario.owner, scenario.bob.id().as_str(), TOKEN_UNIT, 1)
            .await?,
        "transfer to an unregistered receiver",
    )?;

    let burned = scenario.balance(scenario.alice.id().as_str()).await?;
    let forced = scenario
        .call(
            &scenario.alice,
            "storage_unregister",
            json!({"force": true}),
            1,
        )
        .await?;
    successful(&forced, "forced unregister")?;
    ensure!(
        events(&forced)?
            == vec![expected_event(
                "ft_burn",
                json!({
                    "owner_id": scenario.alice.id(), "amount": burned.to_string(), "memo": "storage unregister",
                })
            )]
    );
    ensure!(forced.json::<bool>()?);
    ensure!(
        scenario
            .storage(scenario.alice.id().as_str())
            .await?
            .is_null()
    );
    ensure!(scenario.balance(scenario.alice.id().as_str()).await? == 0);
    ensure!(scenario.supply().await? == TOTAL_SUPPLY - burned);
    ensure!(
        scenario.balance(scenario.owner.id().as_str()).await?
            + scenario.balance(scenario.receiver.id().as_str()).await?
            == TOTAL_SUPPLY - burned
    );

    let reregistered = scenario
        .call(
            &scenario.alice,
            "storage_deposit",
            json!({"registration_only": true}),
            minimum,
        )
        .await?;
    successful(&reregistered, "register after forced burn")?;
    ensure!(scenario.balance(scenario.alice.id().as_str()).await? == 0);
    ensure!(scenario.supply().await? == TOTAL_SUPPLY - burned);
    Ok(())
}

#[tokio::test]
#[ignore = "requires the token/receiver Wasm builds and a local NEAR sandbox"]
async fn standard_token_storage_transfers_refunds_and_events() -> Result<()> {
    let (scenario, mint) = Scenario::start().await?;
    fixed_supply_and_metadata(&scenario, &mint).await?;
    let minimum = storage_registration_and_refunds(&scenario).await?;
    ordinary_transfers(&scenario).await?;
    transfer_call_refunds(&scenario).await?;
    unregister_and_burn(&scenario, minimum).await?;
    Ok(())
}
