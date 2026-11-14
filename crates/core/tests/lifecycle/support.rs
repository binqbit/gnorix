//! Deterministic, deliberately insecure fixtures for public API contract tests.
//!
//! Capture bytes are literal candidate identifiers, not image measurements. The
//! toy signing key is published as its public key and anyone can forge its
//! signatures. These doubles provide no biometric or cryptographic evidence.

use std::{
    cell::Cell,
    error::Error,
    fmt,
    num::{NonZeroU64, NonZeroUsize},
    rc::Rc,
};

use gnorix_core::{
    authorization::{self, Action, Authorization, EnrollmentStatus, VerifiedAuthorization},
    biometrics::{Candidate, Candidates, Capture, MeasurementEngine},
    crypto::{CryptoProvider, SecretInput, SignatureVerifier},
    enrollment::{self, Domain, Enrollment, EnrollmentSpec},
    error::AlgorithmError,
    merkle::Hash,
    policy::{Limits, SecretPolicy},
};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug)]
pub struct FixtureError(pub &'static str);

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "injected test failure: {}", self.0)
    }
}

impl Error for FixtureError {}

#[derive(Clone, Copy)]
pub enum MeasurementFault {
    Prepare,
    EnrollmentStart,
    EnrollmentItem(usize),
    ReconstructionStart,
    ReconstructionItem(usize),
}

pub struct SyntheticMeasurements {
    pub profile: Hash,
    pub fault: Option<MeasurementFault>,
    pub empty_enrollment: bool,
    pub enrollment_yielded: Cell<usize>,
    pub reconstruction_yielded: Cell<usize>,
}

impl SyntheticMeasurements {
    pub fn new() -> Self {
        Self {
            profile: [41; 32],
            fault: None,
            empty_enrollment: false,
            enrollment_yielded: Cell::new(0),
            reconstruction_yielded: Cell::new(0),
        }
    }
}

pub struct SyntheticModel {
    symbols: Vec<u8>,
}

impl MeasurementEngine for SyntheticMeasurements {
    type Model = SyntheticModel;

    fn profile_id(&self) -> Hash {
        self.profile
    }

    fn prepare(&self, captures: &[Capture], _: &Limits) -> Result<Self::Model, AlgorithmError> {
        if matches!(self.fault, Some(MeasurementFault::Prepare)) {
            return Err(Box::new(FixtureError("prepare")));
        }

        Ok(SyntheticModel {
            symbols: captures
                .iter()
                .flat_map(|capture| capture.pixels().iter().copied())
                .collect(),
        })
    }

    fn enrollment_candidates<'a>(
        &'a self,
        model: &'a Self::Model,
        _: &Limits,
    ) -> Result<Candidates<'a>, AlgorithmError> {
        if matches!(self.fault, Some(MeasurementFault::EnrollmentStart)) {
            return Err(Box::new(FixtureError("enrollment start")));
        }

        if self.empty_enrollment {
            return Ok(Box::new(std::iter::empty()));
        }

        let fail_at = match self.fault {
            Some(MeasurementFault::EnrollmentItem(index)) => Some(index),
            _ => None,
        };

        Ok(candidate_stream(
            &model.symbols,
            &self.enrollment_yielded,
            fail_at,
            "enrollment item",
        ))
    }

    fn reconstruct<'a>(
        &'a self,
        capture: &'a Capture,
        _: &'a Self::Model,
        _: &Limits,
    ) -> Result<Candidates<'a>, AlgorithmError> {
        if matches!(self.fault, Some(MeasurementFault::ReconstructionStart)) {
            return Err(Box::new(FixtureError("reconstruction start")));
        }

        let fail_at = match self.fault {
            Some(MeasurementFault::ReconstructionItem(index)) => Some(index),
            _ => None,
        };

        Ok(candidate_stream(
            capture.pixels(),
            &self.reconstruction_yielded,
            fail_at,
            "reconstruction item",
        ))
    }
}

fn candidate_stream<'a>(
    symbols: &'a [u8],
    yielded: &'a Cell<usize>,
    fail_at: Option<usize>,
    label: &'static str,
) -> Candidates<'a> {
    Box::new(symbols.iter().enumerate().map(move |(index, symbol)| {
        yielded.set(yielded.get() + 1);

        if fail_at == Some(index) {
            return Err(Box::new(FixtureError(label)) as AlgorithmError);
        }

        Ok(Candidate::new(vec![*symbol]).unwrap())
    }))
}

#[derive(Clone, Copy)]
pub enum CryptoFault {
    Derive,
    PublicKey,
    Sign,
    Verify,
}

#[derive(Default)]
pub struct KeyCounts {
    pub created: Cell<usize>,
    pub dropped: Cell<usize>,
}

pub struct ToySigningKey {
    bytes: Hash,
    counts: Rc<KeyCounts>,
}

impl Drop for ToySigningKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
        self.counts.dropped.set(self.counts.dropped.get() + 1);
    }
}

impl ZeroizeOnDrop for ToySigningKey {}

pub struct SyntheticCrypto {
    pub suite: &'static str,
    pub fault: Option<CryptoFault>,
    pub keys: Rc<KeyCounts>,
    pub signing_calls: Cell<usize>,
    pub verification_calls: Cell<usize>,
}

impl SyntheticCrypto {
    pub fn new() -> Self {
        Self {
            suite: "test-only-publicly-forgeable-suite",
            fault: None,
            keys: Rc::new(KeyCounts::default()),
            signing_calls: Cell::new(0),
            verification_calls: Cell::new(0),
        }
    }

    pub fn assert_keys_dropped(&self) {
        assert_eq!(self.keys.created.get(), self.keys.dropped.get());
    }
}

impl CryptoProvider for SyntheticCrypto {
    type SigningKey = ToySigningKey;

    fn derive_key(
        &self,
        candidate: &Candidate,
        context: &[u8],
        secret: Option<&SecretInput>,
    ) -> Result<Self::SigningKey, AlgorithmError> {
        if matches!(self.fault, Some(CryptoFault::Derive)) {
            return Err(Box::new(FixtureError("derive")));
        }

        let bytes = toy_digest(&[
            b"test-only-insecure-key",
            candidate.as_bytes(),
            context,
            secret.map_or(&[], SecretInput::as_bytes),
        ]);
        self.keys.created.set(self.keys.created.get() + 1);

        Ok(ToySigningKey {
            bytes,
            counts: Rc::clone(&self.keys),
        })
    }

    fn public_key(&self, key: &Self::SigningKey) -> Result<Vec<u8>, AlgorithmError> {
        if matches!(self.fault, Some(CryptoFault::PublicKey)) {
            return Err(Box::new(FixtureError("public key")));
        }

        Ok(key.bytes.to_vec())
    }

    fn sign(&self, key: &Self::SigningKey, message: &[u8]) -> Result<Vec<u8>, AlgorithmError> {
        self.signing_calls.set(self.signing_calls.get() + 1);

        if matches!(self.fault, Some(CryptoFault::Sign)) {
            return Err(Box::new(FixtureError("sign")));
        }

        Ok(toy_digest(&[b"test-only-forgeable-signature", &key.bytes, message]).to_vec())
    }
}

impl SignatureVerifier for SyntheticCrypto {
    fn suite_id(&self) -> &str {
        self.suite
    }

    fn verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<bool, AlgorithmError> {
        self.verification_calls
            .set(self.verification_calls.get() + 1);

        if matches!(self.fault, Some(CryptoFault::Verify)) {
            return Err(Box::new(FixtureError("verify")));
        }

        Ok(signature == toy_digest(&[b"test-only-forgeable-signature", public_key, message]))
    }
}

fn toy_digest(parts: &[&[u8]]) -> Hash {
    let mut hasher = Sha256::new();

    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }

    hasher.finalize().into()
}

pub fn capture(symbols: &[u8]) -> Capture {
    Capture::new(
        u32::try_from(symbols.len()).unwrap(),
        1,
        symbols.to_vec(),
        None,
        None,
    )
    .unwrap()
}

pub fn limits() -> Limits {
    // Small explicit fixture capacities, not recommended application settings.
    Limits {
        candidate_attempts: NonZeroUsize::new(8).unwrap(),
        candidate_bytes: NonZeroUsize::new(32).unwrap(),
        public_key_bytes: NonZeroUsize::new(32).unwrap(),
        signature_bytes: NonZeroUsize::new(32).unwrap(),
        action_bytes: NonZeroUsize::new(1_024).unwrap(),
        context_bytes: NonZeroUsize::new(1_024).unwrap(),
    }
}

pub fn spec(secret_policy: SecretPolicy) -> EnrollmentSpec {
    EnrollmentSpec {
        domain: Domain {
            network: "test-network".into(),
            verifier: "test-application".into(),
        },
        subject: "test-subject".into(),
        id: [42; 32],
        revision: NonZeroU64::new(1).unwrap(),
        public_salt: b"fixed-test-salt".to_vec(),
        secret_policy,
    }
}

pub fn action() -> Action {
    Action {
        operation: "test-operation".into(),
        payload: b"test-payload".to_vec(),
        nonce: [43; 32],
        expires_at: 100,
    }
}

pub struct Fixture {
    pub measurement: SyntheticMeasurements,
    pub crypto: SyntheticCrypto,
    pub enrollment: Enrollment<SyntheticModel>,
    pub limits: Limits,
    pub action: Action,
}

impl Fixture {
    pub fn new() -> Self {
        let measurement = SyntheticMeasurements::new();
        let crypto = SyntheticCrypto::new();
        let limits = limits();
        let enrollment = enrollment::enroll(
            &measurement,
            &crypto,
            &[capture(&[10, 20, 30])],
            spec(SecretPolicy::Absent),
            None,
            &limits,
        )
        .unwrap();

        Self {
            measurement,
            crypto,
            enrollment,
            limits,
            action: action(),
        }
    }

    pub fn sign(&self, symbols: &[u8]) -> gnorix_core::Result<Authorization> {
        authorization::sign(
            &self.measurement,
            &self.crypto,
            &self.enrollment,
            &capture(symbols),
            self.action.clone(),
            None,
            &self.limits,
        )
    }

    pub fn verify<'a>(
        &'a self,
        authorization: &'a Authorization,
    ) -> gnorix_core::Result<VerifiedAuthorization<'a>> {
        authorization::verify(
            &self.crypto,
            self.enrollment.commitment(),
            EnrollmentStatus::Active,
            &self.action,
            authorization,
            10,
            &self.limits,
        )
    }
}

pub fn failure<T>(result: gnorix_core::Result<T>) -> gnorix_core::Error {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected operation to fail"),
    }
}
