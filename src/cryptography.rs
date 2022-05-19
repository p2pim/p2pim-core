use log::trace;
use rs_merkle::{Hasher, MerkleProof};
use sha3::digest::{FixedOutput, FixedOutputReset};
use sha3::{Digest, Keccak256};

pub const BLOCK_SIZE_BYTES: usize = 544;

pub trait MerkleTree {
  fn append_data<T: AsRef<[u8]>>(&mut self, data: T);
  fn root(&mut self) -> [u8; 32];
  fn proof(&mut self, leaf_index: usize) -> Vec<[u8; 32]>;
}

pub trait Service: Send + Sync + Unpin + Clone + 'static {
  type MerkleTreeType: MerkleTree;
  fn new_merkle_tree() -> Self::MerkleTreeType;
  fn verify(leaf_index: usize, block_data: &[u8], proof: Vec<[u8; 32]>, merkle_root: [u8; 32], total_size: usize) -> bool;
}

pub fn new_service() -> impl Service {
  Implementation
}

#[derive(Clone)]
struct Implementation;

impl Service for Implementation {
  type MerkleTreeType = RsMerkleTree;

  fn new_merkle_tree() -> Self::MerkleTreeType {
    RsMerkleTree {
      inner: rs_merkle::MerkleTree::<Keccak256Hasher>::new(),
      digest: Keccak256::new(),
      current_bytes: 0,
    }
  }

  fn verify(leaf_index: usize, block_data: &[u8], proof: Vec<[u8; 32]>, merkle_root: [u8; 32], total_size: usize) -> bool {
    let merkle_proof = MerkleProof::<Keccak256Hasher>::new(proof.clone());
    let indexes = [leaf_index];
    let leaf_hashes = [Keccak256Hasher::hash(block_data)];
    let total_leaves_count = total_size / BLOCK_SIZE_BYTES + (if total_size % BLOCK_SIZE_BYTES == 0 { 0 } else { 1 });

    trace!(
      "verifying proof merkle_root={} proof={} leaf_hash={} block_data={} total_leaves_count={}",
      hex::encode(merkle_root),
      proof.iter().map(hex::encode).collect::<Vec<String>>().join(","),
      hex::encode(leaf_hashes[0]),
      hex::encode(block_data),
      total_leaves_count
    );
    merkle_proof.verify(
      merkle_root,
      indexes.as_slice(),
      leaf_hashes.as_slice(),
      total_size / BLOCK_SIZE_BYTES + (if total_size % BLOCK_SIZE_BYTES == 0 { 0 } else { 1 }),
    )
  }
}

struct RsMerkleTree {
  inner: rs_merkle::MerkleTree<Keccak256Hasher>,
  digest: Keccak256,
  current_bytes: usize,
}

#[derive(Clone)]
struct Keccak256Hasher;

impl Hasher for Keccak256Hasher {
  type Hash = [u8; 32];

  fn hash(data: &[u8]) -> Self::Hash {
    let mut result = Self::Hash::default();
    result.copy_from_slice(Keccak256::digest(data).as_slice());
    result
  }
}

impl MerkleTree for RsMerkleTree {
  fn append_data<T: AsRef<[u8]>>(&mut self, data: T) {
    let remaining = BLOCK_SIZE_BYTES - self.current_bytes % BLOCK_SIZE_BYTES;

    let mut current = data.as_ref();
    while !current.is_empty() {
      let (left, right) = current.split_at(std::cmp::min(remaining, current.len()));
      self.digest.update(left);

      if self.current_bytes % BLOCK_SIZE_BYTES == 0 {
        let output = self.digest.finalize_fixed_reset();
        let mut result: [u8; 32] = Default::default();
        result.copy_from_slice(output.as_slice());

        trace!("adding leaf hash={} remining_bytes={}", hex::encode(result), right.len());
        self.inner.insert(result);
      }

      current = right;
    }
  }

  fn root(&mut self) -> [u8; 32] {
    if self.current_bytes & BLOCK_SIZE_BYTES != 0 {
      let digest_clone = self.digest.clone();
      let output = digest_clone.finalize_fixed();
      let mut result: [u8; 32] = Default::default();
      result.copy_from_slice(output.as_slice());
      trace!("adding leaf hash={}", hex::encode(result));
      self.inner.insert(result);
    }

    let result = self.inner.uncommitted_root().expect("error geting merkle root");
    self.inner.abort_uncommitted();
    trace!("merkle root hash={}", hex::encode(result));
    result
  }

  fn proof(&mut self, leaf_index: usize) -> Vec<[u8; 32]> {
    // TODO refactorL not very efficient, same code than other
    if self.current_bytes & BLOCK_SIZE_BYTES != 0 {
      let digest_clone = self.digest.clone();
      let output = digest_clone.finalize_fixed();
      let mut result: [u8; 32] = Default::default();
      result.copy_from_slice(output.as_slice());
      self.inner.insert(result);
    }
    let mut other = self.inner.clone();
    other.commit();
    other.proof(&[leaf_index]).proof_hashes().to_vec()
  }
}
