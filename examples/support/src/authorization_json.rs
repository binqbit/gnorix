//! Bounded, typed JSON for actions and public authorization evidence.
//!
//! The sender supplies an enrollment reference, public key, path and signature.
//! A root or replacement context is never accepted from this document. Decoding
//! checks structure and bounds; cryptographic verification remains a core step.

use std::{io::Read, num::NonZeroU64};

use gnorix::{
    authorization::{Action, Authorization},
    merkle::{Hash, MerkleProof},
    policy::Limits,
};
use serde_json::{Map, Value, json};

use crate::Result;

const SCHEMA: &str = "gnorix.authorization.v1";
const ACTION_FIELDS: &[&str] = &["operation", "payload", "nonce", "expires_at"];

pub fn read_action(reader: &mut impl Read, max_bytes: usize, limits: &Limits) -> Result<Action> {
    let read_limit = u64::try_from(max_bytes)?
        .checked_add(1)
        .ok_or("action JSON byte limit overflows")?;
    let mut bytes = Vec::new();
    reader.take(read_limit).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err("action JSON exceeds its byte limit".into());
    }
    parse_action(&serde_json::from_slice(&bytes)?, limits.action_bytes.get())
}

pub fn encode_authorization(authorization: &Authorization) -> Value {
    json!({
        "schema": SCHEMA,
        "enrollment_id": hex(&authorization.enrollment_id),
        "revision": authorization.revision.get(),
        "action": action_value(&authorization.action),
        "public_key": hex(&authorization.public_key),
        "proof": {
            "leaf_index": authorization.proof.leaf_index,
            "leaf_count": authorization.proof.leaf_count,
            "siblings": authorization.proof.siblings.iter().map(|hash| hex(hash)).collect::<Vec<_>>(),
        },
        "signature": hex(&authorization.signature),
    })
}

pub fn decode_authorization(value: &Value, limits: &Limits) -> Result<Authorization> {
    let fields = object(
        value,
        &[
            "schema",
            "enrollment_id",
            "revision",
            "action",
            "public_key",
            "proof",
            "signature",
        ],
    )?;
    if text(field(fields, "schema")?)? != SCHEMA {
        return Err("unsupported authorization schema".into());
    }
    let revision =
        NonZeroU64::new(number(field(fields, "revision")?)?).ok_or("revision must be nonzero")?;
    Ok(Authorization {
        enrollment_id: hash(field(fields, "enrollment_id")?)?,
        revision,
        action: parse_action(field(fields, "action")?, limits.action_bytes.get())?,
        public_key: nonempty_bytes(field(fields, "public_key")?, limits.public_key_bytes.get())?,
        proof: parse_proof(field(fields, "proof")?)?,
        signature: nonempty_bytes(field(fields, "signature")?, limits.signature_bytes.get())?,
    })
}

fn action_value(action: &Action) -> Value {
    json!({
        "operation": action.operation,
        "payload": hex(&action.payload),
        "nonce": hex(&action.nonce),
        "expires_at": action.expires_at,
    })
}

fn parse_action(value: &Value, max_bytes: usize) -> Result<Action> {
    let fields = object(value, ACTION_FIELDS)?;
    let operation = text(field(fields, "operation")?)?;
    if operation.is_empty() {
        return Err("action operation must be nonempty".into());
    }
    let payload = bytes(field(fields, "payload")?, max_bytes)?;
    // Match the core's length-framed domain, operation, payload, nonce and expiry.
    (5 * 8 + b"gnorix.action.v1".len() + 32 + 8)
        .checked_add(operation.len())
        .and_then(|size| size.checked_add(payload.len()))
        .filter(|&size| size <= max_bytes)
        .ok_or("encoded action exceeds its byte limit")?;
    Ok(Action {
        operation: operation.to_owned(),
        payload,
        nonce: hash(field(fields, "nonce")?)?,
        expires_at: number(field(fields, "expires_at")?)?,
    })
}

fn parse_proof(value: &Value) -> Result<MerkleProof> {
    let fields = object(value, &["leaf_index", "leaf_count", "siblings"])?;
    let leaf_index = number(field(fields, "leaf_index")?)?;
    let leaf_count = number(field(fields, "leaf_count")?)?;
    if leaf_count == 0 || leaf_index >= leaf_count {
        return Err("Merkle leaf index must fit a nonempty tree".into());
    }
    let siblings = field(fields, "siblings")?
        .as_array()
        .ok_or("Merkle siblings must be an array")?;
    let depth = (u64::BITS - (leaf_count - 1).leading_zeros()) as usize;
    if siblings.len() > 64 || siblings.len() != depth {
        return Err("Merkle path length does not match its leaf count".into());
    }
    Ok(MerkleProof {
        leaf_index,
        leaf_count,
        siblings: siblings.iter().map(hash).collect::<Result<_>>()?,
    })
}

fn object<'a>(value: &'a Value, expected: &[&str]) -> Result<&'a Map<String, Value>> {
    let fields = value.as_object().ok_or("expected a JSON object")?;
    if fields.len() != expected.len()
        || fields.keys().any(|name| !expected.contains(&name.as_str()))
    {
        return Err("JSON object must contain exactly the documented fields".into());
    }
    Ok(fields)
}

fn field<'a>(fields: &'a Map<String, Value>, name: &str) -> Result<&'a Value> {
    fields
        .get(name)
        .ok_or_else(|| format!("missing {name}").into())
}

fn text(value: &Value) -> Result<&str> {
    value
        .as_str()
        .ok_or_else(|| "expected a JSON string".into())
}

fn number(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| "expected an unsigned integer".into())
}

fn nonempty_bytes(value: &Value, limit: usize) -> Result<Vec<u8>> {
    let decoded = bytes(value, limit)?;
    if decoded.is_empty() {
        return Err("key and signature bytes must be nonempty".into());
    }
    Ok(decoded)
}

fn hash(value: &Value) -> Result<Hash> {
    bytes(value, 32)?
        .try_into()
        .map_err(|_| "hash must encode exactly 32 bytes".into())
}

fn bytes(value: &Value, limit: usize) -> Result<Vec<u8>> {
    let encoded = text(value)?;
    if encoded.len() / 2 > limit {
        return Err("hexadecimal field exceeds its decoded byte limit".into());
    }
    if !encoded.len().is_multiple_of(2)
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("binary fields must use complete lowercase ASCII hex pairs".into());
    }
    Ok(encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (hex_digit(pair[0]) << 4) | hex_digit(pair[1]))
        .collect())
}

fn hex_digit(byte: u8) -> u8 {
    if byte.is_ascii_digit() {
        byte - b'0'
    } else {
        byte - b'a' + 10
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|&byte| {
            [
                char::from(DIGITS[usize::from(byte >> 4)]),
                char::from(DIGITS[usize::from(byte & 15)]),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;

    fn limits() -> Limits {
        Limits {
            candidate_attempts: NonZeroUsize::new(100).unwrap(),
            candidate_bytes: NonZeroUsize::new(32).unwrap(),
            public_key_bytes: NonZeroUsize::new(32).unwrap(),
            signature_bytes: NonZeroUsize::new(64).unwrap(),
            action_bytes: NonZeroUsize::new(4096).unwrap(),
            context_bytes: NonZeroUsize::new(4096).unwrap(),
        }
    }

    fn authorization() -> Authorization {
        Authorization {
            enrollment_id: [3; 32],
            revision: NonZeroU64::new(2).unwrap(),
            action: Action {
                operation: "sign/\u{1f512}".into(),
                payload: vec![0, 255, 128, 1, b'\n', b'\"'],
                nonce: [4; 32],
                expires_at: u64::MAX,
            },
            public_key: vec![5; 32],
            proof: MerkleProof {
                leaf_index: 2,
                leaf_count: 3,
                siblings: vec![[6; 32], [7; 32]],
            },
            signature: vec![8; 64],
        }
    }

    #[test]
    fn round_trip_preserves_arbitrary_binary_payload_and_public_evidence() {
        let original = authorization();
        let encoded = encode_authorization(&original);
        assert_eq!(encoded["schema"], SCHEMA);
        assert_eq!(encoded["action"]["payload"], "00ff80010a22");
        assert_eq!(decode_authorization(&encoded, &limits()).unwrap(), original);
        let action = serde_json::to_vec(&encoded["action"]).unwrap();
        assert_eq!(
            read_action(&mut action.as_slice(), action.len(), &limits()).unwrap(),
            original.action
        );
    }

    #[test]
    fn action_reader_rejects_excess_bytes_malformed_json_and_wrong_fields() {
        let encoded = serde_json::to_vec(&action_value(&authorization().action)).unwrap();
        assert!(read_action(&mut encoded.as_slice(), encoded.len() - 1, &limits()).is_err());
        for bytes in [b"{".as_slice(), b"null", b"[]", b"{}", b"{} {}"] {
            assert!(read_action(&mut &bytes[..], 4096, &limits()).is_err());
        }
        for (field, invalid) in [
            ("operation", json!("")),
            ("payload", json!("0")),
            ("nonce", json!("00")),
            ("expires_at", json!(-1)),
            ("expires_at", json!(1.5)),
            ("root", json!("00")),
        ] {
            let mut action = action_value(&authorization().action);
            action[field] = invalid;
            let bytes = serde_json::to_vec(&action).unwrap();
            assert!(
                read_action(&mut bytes.as_slice(), 4096, &limits()).is_err(),
                "accepted {field}"
            );
        }
    }

    #[test]
    fn rejects_unknown_missing_and_malformed_authorization_fields() {
        let original = encode_authorization(&authorization());
        for field in original.as_object().unwrap().keys() {
            let mut missing = original.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(decode_authorization(&missing, &limits()).is_err());
        }
        for (field, invalid) in [
            ("schema", json!("other")),
            ("revision", json!(0)),
            ("revision", json!(1.5)),
            ("revision", json!("2")),
            ("enrollment_id", json!("00")),
            ("public_key", json!("")),
            ("signature", json!("")),
            ("root", json!("00")),
            ("context", json!({})),
            ("public_key", json!("😃")),
            ("signature", json!("AA")),
        ] {
            let mut changed = original.clone();
            changed[field] = invalid;
            assert!(
                decode_authorization(&changed, &limits()).is_err(),
                "accepted {field}"
            );
        }
    }

    #[test]
    fn limits_cover_decoded_keys_signatures_and_exact_framed_action_size() {
        let original = authorization();
        let encoded = encode_authorization(&original);
        let mut bounded = limits();
        bounded.public_key_bytes = NonZeroUsize::new(31).unwrap();
        assert!(decode_authorization(&encoded, &bounded).is_err());
        bounded = limits();
        bounded.signature_bytes = NonZeroUsize::new(63).unwrap();
        assert!(decode_authorization(&encoded, &bounded).is_err());
        bounded = limits();
        let size = 96 + original.action.operation.len() + original.action.payload.len();
        let action_json = serde_json::to_vec(&encoded["action"]).unwrap();
        bounded.action_bytes = NonZeroUsize::new(size).unwrap();
        assert_eq!(decode_authorization(&encoded, &bounded).unwrap(), original);
        assert_eq!(
            read_action(&mut action_json.as_slice(), 4096, &bounded).unwrap(),
            original.action
        );
        bounded.action_bytes = NonZeroUsize::new(size - 1).unwrap();
        assert!(decode_authorization(&encoded, &bounded).is_err());
        assert!(read_action(&mut action_json.as_slice(), 4096, &bounded).is_err());
    }

    #[test]
    fn merkle_structure_rejects_invalid_numbers_lengths_and_extra_fields() {
        let original = encode_authorization(&authorization());
        for (field, invalid) in [
            ("leaf_index", json!(3)),
            ("leaf_index", json!(-1)),
            ("leaf_count", json!(0)),
            ("leaf_count", json!(1.5)),
            ("siblings", json!([])),
            ("siblings", json!(["00", "00"])),
            ("siblings", json!(vec!["00".repeat(32); 65])),
            ("root", json!("00")),
        ] {
            let mut changed = original.clone();
            changed["proof"][field] = invalid;
            assert!(
                decode_authorization(&changed, &limits()).is_err(),
                "accepted {field}"
            );
        }
        let mut largest = original;
        largest["proof"] = json!({"leaf_index":u64::MAX-1, "leaf_count":u64::MAX,
            "siblings":vec!["00".repeat(32);64]});
        assert_eq!(
            decode_authorization(&largest, &limits())
                .unwrap()
                .proof
                .siblings
                .len(),
            64
        );
    }

    #[test]
    fn valid_tampering_is_preserved_for_core_verification_instead_of_normalized_away() {
        let original = authorization();
        let mut changed = encode_authorization(&original);
        changed["action"]["payload"] = json!("00ff80010a23");
        changed["signature"] = json!("09".repeat(64));
        let decoded = decode_authorization(&changed, &limits()).unwrap();
        assert_ne!(decoded.action.payload, original.action.payload);
        assert_ne!(decoded.signature, original.signature);
        assert_eq!(decoded.public_key, original.public_key);
    }
}
