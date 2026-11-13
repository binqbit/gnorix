//! Portable enrollment context and retained Merkle hashes in the `.grx` format.
//!
//! A package contains no candidate coordinates, candidate digests, private keys,
//! independent secrets or public-key list. Its sorted leaf hashes reconstruct
//! all tree levels and membership paths. Decoding checks internal consistency;
//! the application must still establish trust in the enrollment commitment.

mod codec;

#[cfg(test)]
mod tests;

use std::{error::Error as StdError, fmt, num::NonZeroUsize};

use crate::{
    Error,
    crypto::validate_public_key,
    enrollment::{Enrollment, EnrollmentCommitment},
    merkle::{MerkleProof, MerkleTree, leaf_hash},
    policy::Limits,
};

/// Caller-selected bounds for decoding, encoding and rebuilding retained trees.
#[derive(Clone, Copy, Debug)]
pub struct PackageLimits {
    pub max_bytes: NonZeroUsize,
    pub max_leaves: NonZeroUsize,
}

/// Public enrollment metadata and the hashes needed to find membership proofs.
/// The acquisition/reconstruction model is deliberately separate from this pack.
#[derive(Debug)]
pub struct EnrollmentPackage {
    commitment: EnrollmentCommitment,
    tree: MerkleTree,
}

#[derive(Debug)]
pub enum PackageError {
    InvalidFormat(&'static str),
    ByteLimit,
    LeafLimit,
    Core(Error),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(reason) => write!(f, "invalid grx package: {reason}"),
            Self::ByteLimit => f.write_str("grx package exceeds its byte limit"),
            Self::LeafLimit => f.write_str("grx package exceeds its leaf limit"),
            Self::Core(error) => write!(f, "grx enrollment: {error}"),
        }
    }
}

impl StdError for PackageError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            _ => None,
        }
    }
}

impl From<Error> for PackageError {
    fn from(error: Error) -> Self {
        Self::Core(error)
    }
}

impl EnrollmentPackage {
    /// Retain the commitment and tree, without copying the measurement model.
    pub fn from_enrollment<Model>(
        enrollment: &Enrollment<Model>,
        limits: &Limits,
    ) -> Result<Self, PackageError> {
        enrollment.commitment().context.encode(limits)?;
        Ok(Self {
            commitment: enrollment.commitment().clone(),
            tree: enrollment.proof_material().clone(),
        })
    }

    pub fn commitment(&self) -> &EnrollmentCommitment {
        &self.commitment
    }

    pub(crate) fn proof_material(&self) -> &MerkleTree {
        &self.tree
    }

    /// Compute a leaf from a later public key and locate its membership path.
    /// A valid path proves membership only against a separately trusted root.
    pub fn proof_for(
        &self,
        public_key: &[u8],
        limits: &Limits,
    ) -> Result<MerkleProof, PackageError> {
        validate_public_key(public_key, limits)?;
        let context = self.commitment.context.encode(limits)?;
        let leaf = leaf_hash(&context, public_key);
        self.tree
            .proof_for(&leaf)
            .map_err(Error::from)
            .map_err(Into::into)
    }

    /// Lowercase hexadecimal Merkle root followed by `.grx`.
    pub fn filename(&self) -> String {
        let mut name = String::with_capacity(68);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in self.commitment.root.0 {
            name.push(char::from(HEX[usize::from(byte >> 4)]));
            name.push(char::from(HEX[usize::from(byte & 15)]));
        }
        name.push_str(".grx");
        name
    }

    pub fn encode(
        &self,
        package_limits: &PackageLimits,
        limits: &Limits,
    ) -> Result<Vec<u8>, PackageError> {
        codec::encode(self, package_limits, limits)
    }

    /// Reject unsupported, truncated, noncanonical or inconsistent packages.
    /// The size/count bounds apply before rebuilding tree levels.
    pub fn decode(
        bytes: &[u8],
        package_limits: &PackageLimits,
        limits: &Limits,
    ) -> Result<Self, PackageError> {
        codec::decode(bytes, package_limits, limits)
    }
}
