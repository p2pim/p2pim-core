use std::error::Error;
use std::fmt::{Debug, Formatter};
use std::time::{Duration, SystemTime};
use web3::types::H256;

pub struct Signature(web3::signing::Signature);

impl Debug for Signature {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    f.write_str("TODO Signature")
  }
}

impl From<web3::signing::Signature> for Signature {
  fn from(value: web3::signing::Signature) -> Self {
    Signature(value)
  }
}

impl Signature {
  pub fn serialize(&self) -> Vec<u8> {
    let inner = &self.0;
    // FIXME not very efficient
    let mut result = Vec::new();
    result.append(&mut inner.r.as_bytes().to_vec());
    result.append(&mut inner.s.as_bytes().to_vec());
    // FIXME do we know it is less than 256?
    result.push(inner.v as u8);
    result
  }

  // TODO better error type
  pub fn deserialize(buf: &[u8]) -> Result<Self, Box<dyn Error>> {
    if buf.len() != 65 {
      Err("incorrect input length".into())
    } else {
      Ok(Signature(web3::signing::Signature {
        r: H256::from_slice(&buf[0..32]),
        s: H256::from_slice(&buf[32..64]),
        v: buf[64] as u64,
      }))
    }
  }
}

#[derive(Debug, Clone)]
pub struct Lease {
  pub peer_id: libp2p::PeerId,
  pub peer_address: web3::types::Address,
  pub nonce: u64,
  pub terms: LeaseTerms,
  pub data_parameters: DataParameters,
  pub chain_confirmation: Option<ChainConfirmation>,
}

#[derive(Debug, Clone)]
pub struct DataParameters {
  pub merkle_root: Vec<u8>,
  pub size: usize,
}

#[derive(Debug, Clone)]
pub struct LeaseTerms {
  pub token_address: web3::types::Address,
  pub price: web3::types::U256,
  pub penalty: web3::types::U256,
  pub proposal_expiration: SystemTime,
  pub lease_duration: Duration,
}

#[derive(Debug, Clone)]
pub struct ChainConfirmation {
  pub transaction_hash: web3::types::H256,
  pub timestamp: SystemTime,
}

#[derive(Debug, Clone)]
pub struct Balance {
  pub token_metadata: Option<TokenMetadata>,
  pub storage_balance: StorageBalance,
  pub wallet_balance: WalletBalance,
}

#[derive(Debug, Clone)]
pub struct StorageBalance {
  pub available: web3::types::U256,
  pub locked_rents: web3::types::U256,
  pub locked_lets: web3::types::U256,
}

#[derive(Debug, Clone)]
pub struct WalletBalance {
  pub available: web3::types::U256,
  pub allowance: web3::types::U256,
}

#[derive(Debug, Clone)]
pub struct TokenMetadata {
  pub name: String,
  pub symbol: String,
  pub decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChallengeKey {
  pub nonce: u64,
  pub block_number: u32,
}

#[derive(Debug, Clone)]
pub struct ChallengeProof {
  pub block_data: Vec<u8>,
  pub proof: Vec<[u8; 32]>,
}
