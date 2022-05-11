use crate::types::{ChainConfirmation, Lease};
use libp2p::PeerId;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};
use tonic::async_trait;
use web3::types::Address;

#[derive(Debug)]
pub enum UpdateError {
  LeaseNotFound,
}

impl Display for UpdateError {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      UpdateError::LeaseNotFound => f.write_str("lease not found"),
    }
  }
}

impl Error for UpdateError {}

#[async_trait]
pub trait Service: Clone + Sync + Send + 'static {
  async fn rent_store(&self, lease: Lease);
  async fn rent_update_chain(
    &self,
    peer_address: Address,
    nonce: u64,
    chain_confirmation: Option<ChainConfirmation>,
  ) -> Result<(), UpdateError>;
  async fn rent_list(&self) -> Vec<Lease>;
}

struct Implementation {
  leases_rent: HashMap<Key, Lease>,
}

pub fn new_service() -> impl Service {
  // TODO Make it RwLock
  Arc::new(Mutex::new(Implementation {
    leases_rent: HashMap::new(),
  }))
}

#[async_trait]
impl Service for Arc<Mutex<Implementation>> {
  async fn rent_store(&self, lease: Lease) {
    let mut guard = self.lock().unwrap();
    let key = key(&lease);
    guard.leases_rent.insert(key, lease);
  }

  async fn rent_update_chain(
    &self,
    peer_address: Address,
    nonce: u64,
    chain_confirmation: Option<ChainConfirmation>,
  ) -> Result<(), UpdateError> {
    let mut guard = self.lock().unwrap();

    // TODO unfortunately, we do not have it indexed by peer_address
    let maybe_key = guard
      .leases_rent
      .iter()
      .find(|(_, value)| value.peer_address == peer_address && value.nonce == nonce)
      .map(|(key, value)| (key.clone(), value.clone()));
    match maybe_key {
      None => Err(UpdateError::LeaseNotFound),
      Some((key, mut lease)) => {
        lease.chain_confirmation = chain_confirmation;
        guard.leases_rent.insert(key.clone(), lease);
        Ok(())
      }
    }
  }

  async fn rent_list(&self) -> Vec<Lease> {
    let guard = self.lock().unwrap();
    // TODO should we clone here?
    guard.leases_rent.values().map(|c| c.clone()).collect()
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
  pub peer_id: PeerId,
  pub nonce: u64,
}

fn key(lease: &Lease) -> Key {
  Key {
    peer_id: lease.peer_id.clone(),
    nonce: lease.nonce,
  }
}
