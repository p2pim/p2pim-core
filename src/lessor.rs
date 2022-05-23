use crate::types::LeaseTerms;
use bigdecimal::ToPrimitive;
use libp2p::PeerId;
use log::debug;
use num_bigint::{BigInt, Sign};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::ops::Range;
use std::time::Duration;
use tonic::async_trait;
use web3::types::{Address, U256};

pub enum RejectedReason {
  TokenNotAccepted,
  DurationTooShort,
  DurationTooLong,
  SizeTooSmall,
  SizeTooBig,
  TotalTokensTooSmall,
  PriceRateTooSmall,
  PenaltyRateTooHigh,
}

impl Display for RejectedReason {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      RejectedReason::TokenNotAccepted => f.write_str("token not accepted"),
      RejectedReason::DurationTooShort => f.write_str("duration too short"),
      RejectedReason::DurationTooLong => f.write_str("duration too long"),
      RejectedReason::SizeTooSmall => f.write_str("size too small"),
      RejectedReason::SizeTooBig => f.write_str("size too big"),
      RejectedReason::TotalTokensTooSmall => f.write_str("total tokens too small"),
      RejectedReason::PriceRateTooSmall => f.write_str("price per gb per hour too small"),
      RejectedReason::PenaltyRateTooHigh => f.write_str("penalty too high"),
    }
  }
}

#[derive(Clone, Debug)]
pub struct Ask {
  pub duration_range: Range<Duration>,
  pub size_range: Range<usize>,
  pub min_tokens_total: U256,
  pub min_tokens_gb_hour: U256,
  pub max_penalty_rate: f32,
}

#[async_trait]
pub trait Service: Clone + Sync + Send + 'static {
  async fn proposal(&self, peer_id: &PeerId, lease_terms: &LeaseTerms, size: usize) -> Result<(), RejectedReason>;
}

#[derive(Clone)]
struct Implementation {
  token_ask: HashMap<Address, Ask>,
}

pub fn new_service(token_ask: Vec<(Address, Ask)>) -> impl Service {
  Implementation {
    token_ask: token_ask.into_iter().collect(),
  }
}

#[async_trait]
impl Service for Implementation {
  async fn proposal(&self, _: &PeerId, lease_terms: &LeaseTerms, size: usize) -> Result<(), RejectedReason> {
    if let Some(ask) = self.token_ask.get(&lease_terms.token_address) {
      debug!(
        "checking if proposal is within ask terms lease_terms={:?} ask={:?}",
        lease_terms, ask
      );
      if !ask.duration_range.contains(&lease_terms.lease_duration) {
        if lease_terms.lease_duration < ask.duration_range.start {
          return Err(RejectedReason::DurationTooShort);
        } else {
          return Err(RejectedReason::DurationTooLong);
        }
      }

      if !ask.size_range.contains(&size) {
        if size < ask.size_range.start {
          return Err(RejectedReason::SizeTooSmall);
        } else {
          return Err(RejectedReason::SizeTooBig);
        }
      }

      if lease_terms.price < ask.min_tokens_total {
        return Err(RejectedReason::TotalTokensTooSmall);
      }

      let secs_hour = BigInt::from(3600);
      let bytes_gb = BigInt::from(1024u64 * 1024u64 * 1024u64);
      let mut buf = [0u8; 32];
      lease_terms.price.to_little_endian(buf.as_mut_slice());
      let price_bi = BigInt::from_bytes_le(Sign::Plus, buf.as_slice());
      let tokens_gb_hour = price_bi.clone() * secs_hour * bytes_gb / lease_terms.lease_duration.as_secs() * size;

      if U256::from_little_endian(tokens_gb_hour.to_bytes_le().1.as_slice()) < ask.min_tokens_gb_hour {
        return Err(RejectedReason::PriceRateTooSmall);
      }

      lease_terms.penalty.to_little_endian(buf.as_mut_slice());
      let penalty_bi = BigInt::from_bytes_le(Sign::Plus, buf.as_slice());

      if ask.max_penalty_rate < penalty_bi.to_f32().unwrap() / price_bi.to_f32().unwrap() {
        return Err(RejectedReason::PenaltyRateTooHigh);
      }

      Ok(())
    } else {
      Err(RejectedReason::TokenNotAccepted)
    }
  }
}
