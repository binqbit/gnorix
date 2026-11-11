//! Deterministic commitments to sets of candidate-key hashes.
//!
//! Leaves are sorted and deduplicated before construction. Each level duplicates
//! its last node when it has an odd number of nodes. The final commitment binds
//! both the top node and the number of distinct leaves, including singleton trees.
//! This module retains hashes only; it does not retain candidate keys or secrets.
//!
//! Internal nodes hash `gnorix.merkle.v1.node || left || right`. Roots hash
//! `gnorix.merkle.v1.root || leaf_count || top`, with the count encoded as an
//! unsigned 64-bit big-endian integer. Both child hashes and the top hash are
//! complete 32-byte digests. [`leaf_hash`] documents the separate leaf framing.

use std::error::Error;
use std::fmt;

use sha2::{Digest, Sha256};

/// A complete SHA-256 digest.
pub type Hash = [u8; 32];

/// A commitment to a nonempty, deterministic set of leaf hashes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MerkleRoot(pub Hash);

/// A bottom-up membership path, including the committed set's size.
///
/// `leaf_index` addresses the sorted, deduplicated leaves. Each level contributes
/// exactly one sibling, including the duplicated sibling at an odd boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleProof {
    /// The leaf's zero-based position in the canonical set.
    pub leaf_index: u64,
    /// The number of distinct leaves in the committed set.
    pub leaf_count: u64,
    /// Sibling hashes, starting at the leaf level.
    pub siblings: Vec<Hash>,
}

/// Retained hash levels supporting membership lookup and proof construction.
#[derive(Clone)]
pub struct MerkleTree {
    levels: Vec<Vec<Hash>>,
    root: MerkleRoot,
}

impl fmt::Debug for MerkleTree {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MerkleTree")
            .field("root", &self.root)
            .field("leaf_count", &self.leaf_count())
            .finish_non_exhaustive()
    }
}

/// A failure to construct a commitment or establish membership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MerkleError {
    /// An enrollment must commit at least one candidate.
    EmptyTree,
    /// The requested leaf does not belong to the retained set.
    MissingLeaf,
    /// The proof is malformed or does not match the supplied root and leaf.
    InvalidProof,
}

impl fmt::Display for MerkleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyTree => "a Merkle tree requires at least one leaf",
            Self::MissingLeaf => "the leaf is not in the Merkle tree",
            Self::InvalidProof => "the Merkle proof is invalid",
        })
    }
}

impl Error for MerkleError {}

impl MerkleTree {
    /// Construct a tree from a set of already domain-separated leaf hashes.
    ///
    /// Input order and repeated leaves do not change the commitment.
    pub fn from_leaves(mut leaves: Vec<Hash>) -> Result<Self, MerkleError> {
        leaves.sort_unstable();
        leaves.dedup();

        let leaf_count = leaves.len();
        let levels = build_levels(leaves)?;
        let top = levels
            .last()
            .and_then(|level| level.first())
            .ok_or(MerkleError::EmptyTree)?;
        let root = commit_root(leaf_count as u64, top);

        Ok(Self { levels, root })
    }

    /// Return the count-bound commitment.
    pub fn root(&self) -> MerkleRoot {
        self.root
    }

    /// Return the number of distinct leaves retained by this tree.
    pub fn leaf_count(&self) -> usize {
        // Construction guarantees a nonempty first level; no mutable level or
        // deserialization interface can bypass that invariant.
        self.levels[0].len()
    }

    /// Find an enrolled leaf and construct its membership path.
    pub fn proof_for(&self, leaf: &Hash) -> Result<MerkleProof, MerkleError> {
        let leaves = &self.levels[0];
        let index = leaves
            .binary_search(leaf)
            .map_err(|_| MerkleError::MissingLeaf)?;
        let siblings = collect_siblings(&self.levels, index)?;

        Ok(MerkleProof {
            leaf_index: index as u64,
            leaf_count: leaves.len() as u64,
            siblings,
        })
    }
}

/// Bind a candidate public key to its enrollment/profile context.
///
/// The preimage is `gnorix.merkle.v1.leaf || context_length || context ||
/// key_length || key`. Both lengths are unsigned 64-bit big-endian byte counts.
/// The caller is responsible for canonically encoding the context and key.
pub fn leaf_hash(context: &[u8], public_key: &[u8]) -> Hash {
    let context_length = (context.len() as u64).to_be_bytes();
    let key_length = (public_key.len() as u64).to_be_bytes();

    hash_parts(&[
        b"gnorix.merkle.v1.leaf",
        &context_length,
        context,
        &key_length,
        public_key,
    ])
}

/// Verify membership against a separately trusted commitment.
///
/// Rejects invalid indices, incorrect path lengths, noncanonical duplicate-last
/// siblings, and any mismatch with the supplied leaf or count-bound root.
pub fn verify(root: MerkleRoot, leaf: Hash, proof: &MerkleProof) -> Result<(), MerkleError> {
    validate_proof_shape(proof)?;
    let top = reconstruct_top(leaf, proof)?;
    let reconstructed_root = commit_root(proof.leaf_count, &top);

    if reconstructed_root != root {
        return Err(MerkleError::InvalidProof);
    }

    Ok(())
}

fn build_levels(leaves: Vec<Hash>) -> Result<Vec<Vec<Hash>>, MerkleError> {
    if leaves.is_empty() {
        return Err(MerkleError::EmptyTree);
    }

    let mut levels = Vec::new();
    let mut level = leaves;

    while level.len() > 1 {
        let parents = build_parent_level(&level);
        levels.push(level);
        level = parents;
    }

    levels.push(level);
    Ok(levels)
}

fn build_parent_level(children: &[Hash]) -> Vec<Hash> {
    children
        .chunks(2)
        .map(|pair| {
            let left = &pair[0];
            let right = pair.get(1).unwrap_or(left);

            hash_node(left, right)
        })
        .collect()
}

fn collect_siblings(levels: &[Vec<Hash>], mut index: usize) -> Result<Vec<Hash>, MerkleError> {
    let branch_count = levels.len().saturating_sub(1);
    let mut siblings = Vec::with_capacity(branch_count);

    for level in levels.iter().take(branch_count) {
        let sibling = level
            .get(index ^ 1)
            .or_else(|| level.get(index))
            .ok_or(MerkleError::InvalidProof)?;
        siblings.push(*sibling);
        index /= 2;
    }

    Ok(siblings)
}

fn validate_proof_shape(proof: &MerkleProof) -> Result<(), MerkleError> {
    if proof.leaf_count == 0 || proof.leaf_index >= proof.leaf_count {
        return Err(MerkleError::InvalidProof);
    }

    let mut width = proof.leaf_count;
    let mut expected_depth = 0;

    while width > 1 {
        width = parent_count(width);
        expected_depth += 1;
    }

    if proof.siblings.len() != expected_depth {
        return Err(MerkleError::InvalidProof);
    }

    Ok(())
}

fn reconstruct_top(mut current: Hash, proof: &MerkleProof) -> Result<Hash, MerkleError> {
    let mut index = proof.leaf_index;
    let mut width = proof.leaf_count;

    for sibling in &proof.siblings {
        current = if index.is_multiple_of(2) {
            if index == width - 1 && *sibling != current {
                return Err(MerkleError::InvalidProof);
            }

            hash_node(&current, sibling)
        } else {
            hash_node(sibling, &current)
        };

        index /= 2;
        width = parent_count(width);
    }

    Ok(current)
}

fn parent_count(child_count: u64) -> u64 {
    child_count / 2 + child_count % 2
}

fn hash_node(left: &Hash, right: &Hash) -> Hash {
    hash_parts(&[b"gnorix.merkle.v1.node", left, right])
}

fn commit_root(leaf_count: u64, top: &Hash) -> MerkleRoot {
    let count = leaf_count.to_be_bytes();

    MerkleRoot(hash_parts(&[b"gnorix.merkle.v1.root", &count, top]))
}

fn hash_parts(parts: &[&[u8]]) -> Hash {
    let mut hasher = Sha256::new();

    for part in parts {
        hasher.update(part);
    }

    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(count: u8) -> Vec<Hash> {
        (0..count)
            .map(|value| leaf_hash(b"test-enrollment", &[value]))
            .collect()
    }

    #[test]
    fn rejects_empty_tree_and_missing_leaf() {
        assert!(matches!(
            MerkleTree::from_leaves(Vec::new()),
            Err(MerkleError::EmptyTree)
        ));

        let tree = MerkleTree::from_leaves(leaves(3)).unwrap();
        assert_eq!(
            tree.proof_for(&leaf_hash(b"test-enrollment", b"absent")),
            Err(MerkleError::MissingLeaf)
        );
    }

    #[test]
    fn commitment_and_proofs_depend_only_on_the_distinct_leaf_set() {
        let original = leaves(5);
        let mut shuffled = original.clone();
        shuffled.reverse();
        shuffled.extend_from_slice(&original);

        let canonical = MerkleTree::from_leaves(original.clone()).unwrap();
        let repeated = MerkleTree::from_leaves(shuffled).unwrap();

        assert_eq!(canonical.root(), repeated.root());
        assert_eq!(repeated.leaf_count(), 5);

        for leaf in original {
            assert_eq!(
                canonical.proof_for(&leaf).unwrap(),
                repeated.proof_for(&leaf).unwrap()
            );
        }
    }

    #[test]
    fn every_leaf_verifies_across_singleton_even_and_odd_trees() {
        for count in 1..=65 {
            let input = leaves(count);
            let tree = MerkleTree::from_leaves(input.clone()).unwrap();

            for leaf in input {
                let proof = tree.proof_for(&leaf).unwrap();
                assert_eq!(proof.leaf_count, u64::from(count));
                assert_eq!(verify(tree.root(), leaf, &proof), Ok(()));
            }
        }
    }

    #[test]
    fn singleton_commitment_binds_its_count_instead_of_returning_the_leaf() {
        let leaf = leaf_hash(b"context", b"key");
        let tree = MerkleTree::from_leaves(vec![leaf]).unwrap();
        let proof = tree.proof_for(&leaf).unwrap();

        assert_ne!(tree.root().0, leaf);
        assert_eq!(proof.leaf_index, 0);
        assert_eq!(proof.leaf_count, 1);
        assert!(proof.siblings.is_empty());
        assert_eq!(verify(tree.root(), leaf, &proof), Ok(()));
    }

    #[test]
    fn rejects_changed_leaf_root_index_count_or_path() {
        let input = leaves(5);
        let tree = MerkleTree::from_leaves(input.clone()).unwrap();
        let leaf = input[0];
        let proof = tree.proof_for(&leaf).unwrap();

        let mut wrong_leaf = leaf;
        wrong_leaf[0] ^= 1;
        assert_eq!(
            verify(tree.root(), wrong_leaf, &proof),
            Err(MerkleError::InvalidProof)
        );

        let mut wrong_root = tree.root();
        wrong_root.0[0] ^= 1;
        assert_eq!(
            verify(wrong_root, leaf, &proof),
            Err(MerkleError::InvalidProof)
        );

        let mut mutations = Vec::new();
        let mut wrong_index = proof.clone();
        wrong_index.leaf_index = (proof.leaf_index + 1) % proof.leaf_count;
        mutations.push(wrong_index);

        let mut out_of_range = proof.clone();
        out_of_range.leaf_index = proof.leaf_count;
        mutations.push(out_of_range);

        let mut wrong_count = proof.clone();
        wrong_count.leaf_count += 1;
        mutations.push(wrong_count);

        let mut zero_count = proof.clone();
        zero_count.leaf_count = 0;
        mutations.push(zero_count);

        let mut shorter = proof.clone();
        shorter.siblings.pop();
        mutations.push(shorter);

        let mut longer = proof.clone();
        longer.siblings.push([0; 32]);
        mutations.push(longer);

        let mut changed_sibling = proof;
        changed_sibling.siblings[0][0] ^= 1;
        mutations.push(changed_sibling);

        for mutation in mutations {
            assert_eq!(
                verify(tree.root(), leaf, &mutation),
                Err(MerkleError::InvalidProof)
            );
        }
    }

    #[test]
    fn rejects_noncanonical_duplicated_sibling_even_with_matching_root() {
        let tree = MerkleTree::from_leaves(leaves(5)).unwrap();
        let leaf = *tree.levels[0].last().unwrap();
        let proof = tree.proof_for(&leaf).unwrap();

        assert_eq!(proof.leaf_index, 4);
        assert_eq!(proof.siblings[0], leaf);

        for duplicated_level in [0, 1] {
            let mut forged_proof = proof.clone();
            forged_proof.siblings[duplicated_level] = [42; 32];

            // Construct the matching root without enforcing duplicate-last
            // shape. Checking only the final hash would accept this path.
            let mut forged_top = leaf;
            let mut index = forged_proof.leaf_index;

            for sibling in &forged_proof.siblings {
                forged_top = if index.is_multiple_of(2) {
                    hash_node(&forged_top, sibling)
                } else {
                    hash_node(sibling, &forged_top)
                };
                index /= 2;
            }

            let forged_root = commit_root(forged_proof.leaf_count, &forged_top);
            assert_eq!(
                verify(forged_root, leaf, &forged_proof),
                Err(MerkleError::InvalidProof)
            );
        }
    }

    #[test]
    fn context_and_key_boundaries_are_unambiguous() {
        assert_ne!(leaf_hash(b"ab", b"c"), leaf_hash(b"a", b"bc"));
        assert_ne!(leaf_hash(b"", b"abc"), leaf_hash(b"abc", b""));
        assert_ne!(
            leaf_hash(b"profile-1", b"key"),
            leaf_hash(b"profile-2", b"key")
        );
    }

    #[test]
    fn untrusted_maximum_counts_and_oversized_paths_fail_without_panicking() {
        let root = MerkleRoot([0; 32]);
        let leaf = [0; 32];

        for leaf_count in [0, 1, 2, u64::MAX - 1, u64::MAX] {
            for leaf_index in [0, 1, u64::MAX - 1, u64::MAX] {
                for path_length in [0, 1, 63, 64, 65, 1_000] {
                    let proof = MerkleProof {
                        leaf_index,
                        leaf_count,
                        siblings: vec![[0; 32]; path_length],
                    };

                    assert_eq!(verify(root, leaf, &proof), Err(MerkleError::InvalidProof));
                }
            }
        }
    }

    #[test]
    fn matches_independently_computed_sha256_vector() {
        // Independently computed using Python's hashlib.sha256 with the byte
        // domains and big-endian lengths documented by this module.
        let expected_leaf = [
            253, 165, 202, 1, 171, 71, 60, 227, 30, 80, 37, 131, 244, 109, 213, 137, 71, 210, 197,
            192, 80, 19, 28, 160, 54, 94, 60, 9, 180, 204, 118, 68,
        ];
        let expected_root = MerkleRoot([
            165, 239, 74, 238, 123, 118, 207, 12, 99, 80, 204, 191, 7, 219, 68, 9, 34, 250, 78, 94,
            32, 172, 65, 130, 219, 71, 170, 252, 37, 56, 187, 43,
        ]);
        let expected_proof = MerkleProof {
            leaf_index: 4,
            leaf_count: 5,
            siblings: vec![
                expected_leaf,
                [
                    161, 7, 202, 213, 112, 251, 154, 135, 182, 154, 46, 135, 34, 227, 40, 43, 192,
                    76, 200, 139, 76, 64, 14, 66, 224, 149, 231, 70, 196, 121, 244, 126,
                ],
                [
                    13, 255, 169, 15, 216, 134, 15, 2, 6, 254, 150, 47, 133, 150, 177, 126, 122,
                    181, 3, 200, 183, 167, 148, 70, 50, 1, 219, 245, 161, 127, 43, 249,
                ],
            ],
        };
        let tree = MerkleTree::from_leaves(
            b"ABCDE"
                .iter()
                .map(|key| leaf_hash(b"profile:v1", &[*key]))
                .collect(),
        )
        .unwrap();

        assert_eq!(leaf_hash(b"profile:v1", b"A"), expected_leaf);
        assert_eq!(tree.root(), expected_root);
        assert_eq!(tree.proof_for(&expected_leaf).unwrap(), expected_proof);
        assert_eq!(
            verify(expected_root, expected_leaf, &expected_proof),
            Ok(())
        );
    }
}
