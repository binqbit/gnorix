//! Explicit example suite, not a library-selected cryptographic default.
//!
//! HKDF is a key derivation primitive, not password hardening or an estimate of
//! biometric entropy. Applications choose their own complete provider policy.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use gnorix::{
    biometrics::Candidate,
    crypto::{CryptoProvider, SecretInput, SignatureVerifier},
    error::AlgorithmError,
};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

pub const SUITE_ID: &str = "gnorix.example.hkdf-sha256-ed25519.v1";
const INPUT_DOMAIN: &[u8] = b"gnorix.example.ed25519-input.v1";
const EXPAND_INFO: &[u8] = b"gnorix.example.ed25519-seed.v1";

pub struct ExampleCrypto;

impl CryptoProvider for ExampleCrypto {
    type SigningKey = SigningKey;

    fn derive_key(
        &self,
        candidate: &Candidate,
        context: &[u8],
        secret: Option<&SecretInput>,
    ) -> Result<SigningKey, AlgorithmError> {
        if candidate.as_bytes().len() != 32 {
            return Err("example suite requires a 32-byte candidate digest".into());
        }
        let input = derivation_input(candidate, secret)?;
        let seed = derive_seed(&input, context)?;
        Ok(SigningKey::from_bytes(&seed))
    }

    fn public_key(&self, key: &SigningKey) -> Result<Vec<u8>, AlgorithmError> {
        Ok(key.verifying_key().to_bytes().to_vec())
    }

    fn sign(&self, key: &SigningKey, message: &[u8]) -> Result<Vec<u8>, AlgorithmError> {
        Ok(key.sign(message).to_bytes().to_vec())
    }
}

impl SignatureVerifier for ExampleCrypto {
    fn suite_id(&self) -> &str {
        SUITE_ID
    }

    fn verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<bool, AlgorithmError> {
        let Ok(public_bytes) = <&[u8; 32]>::try_from(public_key) else {
            return Ok(false);
        };
        let Ok(key) = VerifyingKey::from_bytes(public_bytes) else {
            return Ok(false);
        };
        let Ok(signature) = Signature::try_from(signature) else {
            return Ok(false);
        };
        Ok(key.verify_strict(message, &signature).is_ok())
    }
}

/// The IKM frames domain, candidate, a presence byte and secret independently.
/// Every field is prefixed with its u64 big-endian byte length.
fn derivation_input(
    candidate: &Candidate,
    secret: Option<&SecretInput>,
) -> Result<Zeroizing<Vec<u8>>, AlgorithmError> {
    let presence = [u8::from(secret.is_some())];
    let fields = [
        INPUT_DOMAIN,
        candidate.as_bytes(),
        &presence,
        secret.map_or(&[], SecretInput::as_bytes),
    ];
    let mut input = Zeroizing::new(Vec::new());
    for field in fields {
        let length = u64::try_from(field.len())?;
        input.extend_from_slice(&length.to_be_bytes());
        input.extend_from_slice(field);
    }
    Ok(input)
}

fn derive_seed(input: &[u8], context: &[u8]) -> Result<Zeroizing<[u8; 32]>, AlgorithmError> {
    let (mut prk, hkdf) = Hkdf::<Sha256>::extract(Some(context), input);
    prk.as_mut_slice().zeroize();
    let mut seed = Zeroizing::new([0_u8; 32]);
    hkdf.expand(EXPAND_INFO, seed.as_mut())?;
    Ok(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex<const N: usize>(encoded: &str) -> [u8; N] {
        let mut result = [0; N];
        assert_eq!(encoded.len(), 2 * N);
        for (index, byte) in result.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16).unwrap();
        }
        result
    }

    #[test]
    fn ed25519_matches_rfc8032_empty_message_vector() {
        let key = SigningKey::from_bytes(&hex(
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        ));
        let public = hex::<32>("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let signature = hex::<64>(concat!(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
            "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        ));
        assert_eq!(ExampleCrypto.public_key(&key).unwrap(), public);
        assert_eq!(ExampleCrypto.sign(&key, b"").unwrap(), signature);
        assert!(ExampleCrypto.verify(&public, b"", &signature).unwrap());
        assert!(
            !ExampleCrypto
                .verify(&public, b"changed", &signature)
                .unwrap()
        );
        assert!(!ExampleCrypto.verify(&[0; 31], b"", &signature).unwrap());
        assert!(!ExampleCrypto.verify(&[0; 32], b"", &[0; 64]).unwrap());
    }

    #[test]
    fn hkdf_matches_rfc5869_case_one() {
        let key = Hkdf::<Sha256>::new(Some(&hex::<13>("000102030405060708090a0b0c")), &[0x0b; 22]);
        let mut output = [0; 42];
        key.expand(&hex::<10>("f0f1f2f3f4f5f6f7f8f9"), &mut output)
            .unwrap();
        assert_eq!(
            output,
            hex::<42>(concat!(
                "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf",
                "34007208d5b887185865"
            ))
        );
    }

    #[test]
    fn provider_is_deterministic_and_binds_candidate_context_and_secret() {
        let candidate = Candidate::new(vec![1; 32]).unwrap();
        let other = Candidate::new(vec![2; 32]).unwrap();
        let secret = SecretInput::new(b"test-secret".to_vec()).unwrap();
        let public = |candidate: &Candidate, context: &[u8], secret| {
            let key = ExampleCrypto
                .derive_key(candidate, context, secret)
                .unwrap();
            ExampleCrypto.public_key(&key).unwrap()
        };
        let baseline = public(&candidate, b"context", None);
        assert_eq!(baseline, public(&candidate, b"context", None));
        assert_ne!(baseline, public(&other, b"context", None));
        assert_ne!(baseline, public(&candidate, b"other-context", None));
        assert_ne!(baseline, public(&candidate, b"context", Some(&secret)));
        assert!(
            ExampleCrypto
                .derive_key(&Candidate::new(vec![1]).unwrap(), b"context", None)
                .is_err()
        );
    }

    #[test]
    fn example_framing_and_derivation_match_independent_python_hmac_vectors() {
        // Calculated with Python hashlib/hmac, independent of hkdf's Rust code.
        let candidate = Candidate::new(vec![1; 32]).unwrap();
        let secret = SecretInput::new(b"test-secret".to_vec()).unwrap();
        for (secret, expected) in [
            (
                None,
                "160f7ea0289a78075d1c18df68e052d5dc108e79bbd60d9adf613b4cb0e653a3",
            ),
            (
                Some(&secret),
                "84cbff4ed3e5f827f0ed9827933fbbe30999a3a3d5224811dd254b71246d09dc",
            ),
        ] {
            let input = derivation_input(&candidate, secret).unwrap();
            assert_eq!(
                *derive_seed(&input, b"context").unwrap(),
                hex::<32>(expected)
            );
        }
    }
}
