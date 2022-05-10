use log::debug;
use sha3::digest::Output;
use sha3::{Digest, Keccak256};
use std::cmp;
use std::collections::VecDeque;

const BLOCK_SIZE_BYTES: usize = 272;

pub struct MerkleTree {
  digest: Keccak256,
  state: VecDeque<(u32, Output<Keccak256>)>,
  current_bytes: usize,
}

impl MerkleTree {
  pub fn new() -> Self {
    MerkleTree {
      digest: Keccak256::new(),
      state: VecDeque::new(),
      current_bytes: 0,
    }
  }

  pub fn update(&mut self, data: impl AsRef<[u8]>) {
    let remaining = BLOCK_SIZE_BYTES - self.current_bytes % BLOCK_SIZE_BYTES;
    let len = data.as_ref().len();

    debug!("Remaining bytes: {}, current len: {}", remaining, len);
    if len < remaining {
      self.digest.update(data.as_ref());
      self.current_bytes += data.as_ref().len();
    } else {
      let (current, rest) = data.as_ref().split_at(remaining);
      self.digest.update(current);
      self.current_bytes += current.len();

      let mut result = Default::default();
      self.digest.finalize_into_reset(&mut result);
      log::debug!("Push ({}, {})", 1, hex::encode(result.as_slice()));
      self.state.push_front((1, result.clone()));
      self.collapse_state();
      self.update(rest);
    }
  }

  pub fn finalize(mut self) -> Vec<u8> {
    if self.current_bytes % BLOCK_SIZE_BYTES > 0 {
      let mut result = Default::default();
      self.digest.finalize_into_reset(&mut result);
      log::debug!("Push ({}, {})", 1, hex::encode(result.as_slice()));
      self.state.push_front((1, result.clone()));
      self.collapse_state();
    }

    log::debug!("Finalising... Count: {}", self.state.len());

    let result = self
      .state
      .into_iter()
      .reduce(|left, right| {
        let mut digest = Keccak256::new();
        digest.update(right.1.as_slice());
        digest.update(left.1.as_slice());
        (cmp::max(left.0, right.0), digest.finalize())
      })
      .unwrap_or((0, Keccak256::digest(&[])))
      .1
      .to_vec();
    debug!("Merkle root: {}", hex::encode(result.as_slice()));
    result
  }

  fn collapse_state(&mut self) {
    if self.state.len() > 1 {
      let left = self.state.pop_front().unwrap();
      let right = self.state.pop_front().unwrap();
      if left.0 == right.0 {
        log::debug!("Pop ({}, {})", left.0, hex::encode(left.1.as_slice()));
        log::debug!("Pop ({}, {})", right.0, hex::encode(right.1.as_slice()));
        let next_row = left.0 + 1;
        let mut digest = Keccak256::new();
        digest.update(right.1.as_slice());
        digest.update(left.1.as_slice());
        let result = digest.finalize();
        log::debug!("Push ({}, {})", next_row, hex::encode(result.as_slice()));
        self.state.push_front((next_row, result));
        self.collapse_state();
      } else {
        self.state.push_front(right);
        self.state.push_front(left);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use crate::utils::merkle::MerkleTree;
  use core::iter;

  fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
  }

  #[test]
  fn merkle_empty() {
    init();
    let merkle = MerkleTree::new();
    let result = merkle.finalize();
    assert_eq!(
      result,
      hex::decode("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470").unwrap()
    );
  }

  #[test]
  fn merkle_less_one_block() {
    init();
    let mut merkle = MerkleTree::new();
    merkle.update(hex::decode("00").unwrap().as_slice());
    let result = merkle.finalize();
    assert_eq!(
      result,
      hex::decode("bc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a").unwrap()
    );
  }

  #[test]
  fn merkle_exactly_one_block() {
    init();
    let mut merkle = MerkleTree::new();
    let input: Vec<u8> = iter::repeat(0).take(272).collect();
    merkle.update(input.as_slice());
    let result = merkle.finalize();
    assert_eq!(
      hex::encode(result),
      "a8005c7a3125b6c3629b4181eca54d18721e41fef639718d205beb00b366ed7d"
    );
  }

  #[test]
  fn merkle_bigger_than_one_block() {
    init();
    let mut merkle = MerkleTree::new();
    let input: Vec<u8> = iter::repeat(0).take(273).collect();
    merkle.update(input.as_slice());
    let result = merkle.finalize();
    assert_eq!(
      hex::encode(result),
      "b615a67cb2341add303e12887acbc279771120e30ac9bad4d1374c5e2d706d7b"
    );
  }

  #[test]
  fn merkle_three_blocks() {
    init();
    let mut merkle = MerkleTree::new();
    let input: Vec<u8> = iter::repeat(0).take(272 * 3).collect();
    merkle.update(input.as_slice());
    let result = merkle.finalize();
    assert_eq!(
      hex::encode(result),
      "dc738a0a5aeac6217215b2f78b0db996181450e9eee083fc7fba4119bcc7876b"
    );
  }
}
