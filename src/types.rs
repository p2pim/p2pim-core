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

#[derive(Debug)]
pub struct LeaseTerms {
  pub token_address: web3::types::Address,
  pub price: web3::types::U256,
  pub penalty: web3::types::U256,
  pub proposal_expiration: SystemTime,
  pub lease_duration: Duration,
}
