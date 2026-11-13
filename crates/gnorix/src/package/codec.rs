//! Canonical bounded binary framing for public enrollment/proof material.

use std::num::NonZeroU64;

use crate::{
    enrollment::{Domain, EnrollmentCommitment, EnrollmentContext, EnrollmentSpec},
    merkle::{Hash, MerkleRoot, MerkleTree},
    policy::{Limits, SecretPolicy},
};

use super::{EnrollmentPackage, PackageError, PackageLimits, validate_profile_parameters};

const MAGIC: &[u8] = b"GNORIX-GRX\x00\x02";
const CONTEXT_DOMAIN: &[u8] = b"gnorix.enrollment.v1";

pub(super) fn encode(
    package: &EnrollmentPackage,
    capacity: &PackageLimits,
    limits: &Limits,
) -> Result<Vec<u8>, PackageError> {
    let leaves = package.tree.leaves();
    if leaves.len() > capacity.max_leaves.get() {
        return Err(PackageError::LeafLimit);
    }
    let context = package.commitment.context.encode(limits)?;
    let parameters = package.profile_parameters().unwrap_or_default();
    if package.profile_parameters().is_some() {
        validate_profile_parameters(parameters, &package.commitment.context.profile_id, limits)?;
    }
    let length = leaves
        .len()
        .checked_mul(32)
        .and_then(|size| size.checked_add(MAGIC.len() + 8 + 8 + 32 + 8))
        .and_then(|size| size.checked_add(context.len()))
        .and_then(|size| size.checked_add(parameters.len()))
        .filter(|&size| size <= capacity.max_bytes.get())
        .ok_or(PackageError::ByteLimit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| PackageError::ByteLimit)?;
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(context.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&context);
    bytes.extend_from_slice(&(parameters.len() as u64).to_be_bytes());
    bytes.extend_from_slice(parameters);
    bytes.extend_from_slice(&package.commitment.root.0);
    bytes.extend_from_slice(&(leaves.len() as u64).to_be_bytes());
    for leaf in leaves {
        bytes.extend_from_slice(leaf);
    }
    Ok(bytes)
}

pub(super) fn decode(
    bytes: &[u8],
    capacity: &PackageLimits,
    limits: &Limits,
) -> Result<EnrollmentPackage, PackageError> {
    if bytes.len() > capacity.max_bytes.get() {
        return Err(PackageError::ByteLimit);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(invalid("unsupported magic or version"));
    }
    let encoded_context = reader.field()?;
    if encoded_context.len() > limits.context_bytes.get() {
        return Err(PackageError::Core(crate::Error::LimitExceeded(
            crate::error::Resource::ContextBytes,
        )));
    }
    let context = decode_context(encoded_context)?;
    if context.encode(limits)? != encoded_context {
        return Err(invalid("noncanonical enrollment context"));
    }
    let profile_parameters = decode_profile_parameters(&mut reader, &context.profile_id, limits)?;
    let root = MerkleRoot(reader.hash()?);
    let leaf_count = reader.number()?;
    let leaves = decode_leaves(&mut reader, leaf_count, capacity)?;
    reader.finish()?;
    let tree = MerkleTree::from_leaves(leaves).map_err(crate::Error::from)?;
    if tree.root() != root {
        return Err(invalid("leaf hashes do not match the Merkle root"));
    }
    Ok(EnrollmentPackage {
        commitment: EnrollmentCommitment {
            context,
            root,
            leaf_count,
        },
        tree,
        profile_parameters,
    })
}

fn decode_profile_parameters(
    reader: &mut Reader<'_>,
    profile_id: &Hash,
    limits: &Limits,
) -> Result<Option<Vec<u8>>, PackageError> {
    let encoded = reader.field()?;
    if encoded.is_empty() {
        return Ok(None);
    }
    validate_profile_parameters(encoded, profile_id, limits)?;
    let mut parameters = Vec::new();
    parameters
        .try_reserve_exact(encoded.len())
        .map_err(|_| PackageError::ByteLimit)?;
    parameters.extend_from_slice(encoded);
    Ok(Some(parameters))
}

fn decode_context(bytes: &[u8]) -> Result<EnrollmentContext, PackageError> {
    let mut reader = Reader::new(bytes);
    if reader.field()? != CONTEXT_DOMAIN {
        return Err(invalid("unsupported enrollment context"));
    }
    let network = reader.text()?;
    let verifier = reader.text()?;
    let subject = reader.text()?;
    let id = hash(reader.field()?)?;
    let revision_bytes: [u8; 8] = reader
        .field()?
        .try_into()
        .map_err(|_| invalid("invalid revision width"))?;
    let revision = NonZeroU64::new(u64::from_be_bytes(revision_bytes))
        .ok_or(invalid("zero enrollment revision"))?;
    let public_salt = reader.field()?.to_vec();
    let secret_policy = match reader.field()? {
        [0] => SecretPolicy::Absent,
        [1] => SecretPolicy::Required,
        _ => return Err(invalid("unknown secret policy")),
    };
    let profile_id = hash(reader.field()?)?;
    let suite_id = reader.text()?;
    reader.finish()?;
    Ok(EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain { network, verifier },
            subject,
            id,
            revision,
            public_salt,
            secret_policy,
        },
        profile_id,
        suite_id,
    })
}

fn decode_leaves(
    reader: &mut Reader<'_>,
    count: u64,
    capacity: &PackageLimits,
) -> Result<Vec<Hash>, PackageError> {
    if count == 0 {
        return Err(invalid("empty tree"));
    }
    let count = usize::try_from(count).map_err(|_| PackageError::LeafLimit)?;
    if count > capacity.max_leaves.get() {
        return Err(PackageError::LeafLimit);
    }
    let length = count.checked_mul(32).ok_or(PackageError::ByteLimit)?;
    let encoded = reader.take(length)?;
    let mut leaves = Vec::new();
    leaves
        .try_reserve_exact(count)
        .map_err(|_| PackageError::LeafLimit)?;
    for chunk in encoded.as_chunks::<32>().0 {
        let leaf = hash(chunk)?;
        if leaves.last().is_some_and(|previous| previous >= &leaf) {
            return Err(invalid("leaf hashes must be sorted and unique"));
        }
        leaves.push(leaf);
    }
    Ok(leaves)
}

fn hash(bytes: &[u8]) -> Result<Hash, PackageError> {
    bytes.try_into().map_err(|_| invalid("invalid hash width"))
}

fn invalid(reason: &'static str) -> PackageError {
    PackageError::InvalidFormat(reason)
}

struct Reader<'a> {
    remaining: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], PackageError> {
        let (value, remaining) = self
            .remaining
            .split_at_checked(length)
            .ok_or(invalid("truncated field"))?;
        self.remaining = remaining;
        Ok(value)
    }

    fn number(&mut self) -> Result<u64, PackageError> {
        let bytes = self
            .take(8)?
            .try_into()
            .map_err(|_| invalid("invalid number width"))?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn field(&mut self) -> Result<&'a [u8], PackageError> {
        let length = usize::try_from(self.number()?).map_err(|_| PackageError::ByteLimit)?;
        self.take(length)
    }

    fn hash(&mut self) -> Result<Hash, PackageError> {
        hash(self.take(32)?)
    }

    fn text(&mut self) -> Result<String, PackageError> {
        std::str::from_utf8(self.field()?)
            .map(str::to_owned)
            .map_err(|_| invalid("invalid UTF-8 text"))
    }

    fn finish(self) -> Result<(), PackageError> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(invalid("trailing data"))
        }
    }
}
