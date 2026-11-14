use near_sdk::{AccountId, borsh, json_types::U128, near};

/// Application purpose bound by the Gnorix authorization.
pub const TRANSFER_OPERATION: &str = "wallet.ft_transfer.v1";

/// Exact transfer approved by the owner. Amount is in the token's smallest units.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub token_id: AccountId,
    pub receiver_id: AccountId,
    pub amount: U128,
    pub memo: Option<String>,
}

impl Transfer {
    /// Borsh tuple, in this order: token account, receiver account, raw u128
    /// amount, optional UTF-8 memo. JSON property order cannot alter these bytes.
    pub fn signing_payload(&self) -> Vec<u8> {
        borsh::to_vec(&(&self.token_id, &self.receiver_id, self.amount.0, &self.memo))
            .expect("transfer payload serialization failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_canonical_and_binds_every_transfer_argument() {
        let transfer = Transfer {
            token_id: "ft.testnet".parse().unwrap(),
            receiver_id: "bob.testnet".parse().unwrap(),
            amount: U128(513),
            memo: Some("ok".into()),
        };
        let mut expected = vec![10, 0, 0, 0];
        expected.extend_from_slice(b"ft.testnet");
        expected.extend_from_slice(&[11, 0, 0, 0]);
        expected.extend_from_slice(b"bob.testnet");
        expected.extend_from_slice(&513_u128.to_le_bytes());
        expected.extend_from_slice(&[1, 2, 0, 0, 0, b'o', b'k']);
        assert_eq!(transfer.signing_payload(), expected);

        let changes: [fn(&mut Transfer); 4] = [
            |value| value.token_id = "other.testnet".parse().unwrap(),
            |value| value.receiver_id = "eve.testnet".parse().unwrap(),
            |value| value.amount.0 += 1,
            |value| value.memo = None,
        ];
        for change in changes {
            let mut changed = transfer.clone();
            change(&mut changed);
            assert_ne!(changed.signing_payload(), expected);
        }

        let mut no_memo = transfer.clone();
        no_memo.memo = None;
        let mut empty_memo = transfer;
        empty_memo.memo = Some(String::new());
        assert_ne!(no_memo.signing_payload(), empty_memo.signing_payload());
    }
}
