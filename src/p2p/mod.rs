use crate::types::{ChallengeKey, ChallengeProof, LeaseTerms, Signature};
use futures::Stream;
use libp2p::core::Executor;
use libp2p::identity::secp256k1::PublicKey;
use libp2p::identity::{secp256k1, Keypair};
use libp2p::swarm::{SwarmBuilder, SwarmEvent};
use libp2p::{PeerId, Swarm};
use log::{debug, trace};
use std::borrow::Borrow;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tonic::async_trait;

pub mod behaviour;
pub mod p2pim;
pub mod transport;

#[async_trait]
pub trait Service: Stream<Item = behaviour::Event> + Send + Sync + Clone + Unpin + 'static {
  async fn send_proposal(&self, peer_id: PeerId, nonce: u64, terms: LeaseTerms, signature: Signature, data: Vec<u8>);
  async fn send_challenge(&self, peer_id: PeerId, challenge_key: ChallengeKey);
  async fn send_challenge_proof(&self, peer_id: PeerId, challenge_key: ChallengeKey, challenge_proof: ChallengeProof);
  fn find_public_key(&self, peer_id: &PeerId) -> Option<secp256k1::PublicKey>;
  fn known_peers(&self) -> Vec<PeerId>;
}

struct Implementation(Arc<Mutex<Swarm<behaviour::Behaviour>>>);

pub async fn create_p2p(keypair: Keypair) -> Result<impl Service, Box<dyn Error>> {
  let transport = transport::build_transport(keypair.clone())?;
  let behaviour = behaviour::Behaviour::new(keypair.public()).await?;
  let local_peer_id = PeerId::from_public_key(keypair.public().borrow());
  let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
    .executor(Box::new(TokioExecutor {}))
    .build();
  debug!("swarm build with local peer id {}", local_peer_id);
  // TODO Make address parametrized
  swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

  Ok(Implementation(Arc::new(Mutex::new(swarm))))
}

struct TokioExecutor {}

impl Executor for TokioExecutor {
  fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
    tokio::task::spawn(future);
  }
}

impl Stream for Implementation {
  type Item = behaviour::Event;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut guard = self.0.lock().unwrap();
    while let Poll::Ready(e) = futures::stream::StreamExt::poll_next_unpin(&mut *guard, cx) {
      match e {
        Some(SwarmEvent::Behaviour(be)) => {
          return Poll::Ready(Some(be));
        }
        Some(other) => {
          trace!("swarm: {:?}", other)
        }
        None => {
          return Poll::Ready(None);
        }
      }
    }
    Poll::Pending
  }
}

impl Clone for Implementation {
  fn clone(&self) -> Self {
    Implementation(Arc::clone(&self.0))
  }
}

#[async_trait]
impl Service for Implementation {
  async fn send_proposal(&self, peer_id: PeerId, nonce: u64, terms: LeaseTerms, signature: Signature, data: Vec<u8>) {
    let mut guard = self.0.lock().unwrap();
    guard.behaviour_mut().p2pim.send_proposal(
      peer_id,
      p2pim::LeaseProposal {
        nonce,
        lease_terms: terms,
        signature,
        data,
      },
    )
  }

  async fn send_challenge(&self, peer_id: PeerId, challenge_key: ChallengeKey) {
    let mut guard = self.0.lock().unwrap();
    guard.behaviour_mut().p2pim.send_challenge(peer_id, challenge_key)
  }

  async fn send_challenge_proof(&self, peer_id: PeerId, challenge_key: ChallengeKey, challenge_proof: ChallengeProof) {
    let mut guard = self.0.lock().unwrap();
    guard
      .behaviour_mut()
      .p2pim
      .send_challenge_proof(peer_id, challenge_key, challenge_proof);
  }

  fn find_public_key(&self, peer_id: &PeerId) -> Option<PublicKey> {
    let guard = self.0.lock().unwrap();
    guard.behaviour().peer_info(peer_id).and_then(|i| {
      if let libp2p::identity::PublicKey::Secp256k1(p) = i.public_key.clone() {
        Some(p)
      } else {
        None
      }
    })
  }

  fn known_peers(&self) -> Vec<PeerId> {
    let guard = self.0.lock().unwrap();
    guard.behaviour().known_peers()
  }
}
