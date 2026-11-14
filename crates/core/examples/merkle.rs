//! Exercise only the commitment/proof API with illustrative public-key bytes.
//! This example does not capture a fingerprint or derive signing keys.

use gnorix::merkle::{MerkleTree, leaf_hash, verify};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let context = b"example enrollment context";
    let first = leaf_hash(context, b"illustrative public key A");
    let second = leaf_hash(context, b"illustrative public key B");
    let tree = MerkleTree::from_leaves(vec![second, first, first])?;

    let proof = tree.proof_for(&first)?;
    verify(tree.root(), first, &proof)?;

    println!(
        "Verified membership: {} distinct leaves, {} sibling hashes.",
        tree.leaf_count(),
        proof.siblings.len(),
    );

    Ok(())
}
