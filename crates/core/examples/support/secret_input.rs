//! Bounded optional secret input shared by enrollment and signing examples.

use std::env;

use gnorix_core::{crypto::SecretInput, policy::SecretPolicy};

use super::Result;

pub(super) const MAX_SECRET_BYTES: usize = 4096;

pub(super) fn read_secret(policy: SecretPolicy) -> Result<Option<SecretInput>> {
    let secret = match env::var("GNORIX_ENROLL_SECRET") {
        Ok(value) => {
            let secret = SecretInput::new(value.into_bytes())?;
            if secret.as_bytes().len() > MAX_SECRET_BYTES {
                return Err("secret input exceeds the example byte limit".into());
            }
            Some(secret)
        }
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => return Err("secret input must be UTF-8".into()),
    };
    if !matches!(
        (policy, &secret),
        (SecretPolicy::Absent, None) | (SecretPolicy::Required, Some(_))
    ) {
        return Err(gnorix_core::Error::SecretPolicyMismatch.into());
    }
    Ok(secret)
}
