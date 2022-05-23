use crate::p2p::p2pim::LeaseProposal;
use crate::types::{ChallengeKey, ChallengeProof, LeaseTerms, Signature};
use crate::utils::sync::OneshotListerners;
use futures::Stream;
use libp2p::core::Executor;
use libp2p::identity::secp256k1::PublicKey;
use libp2p::identity::{secp256k1, Keypair};
use libp2p::swarm::{SwarmBuilder, SwarmEvent};
use libp2p::{PeerId, Swarm};
use log::{debug, trace, warn};
use std::borrow::Borrow;
use std::error::Error;
use std::future::Future;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tonic::async_trait;

pub mod behaviour;
pub mod p2pim;
pub mod transport;

pub enum Event {
  ReceivedLeaseProposal { peer_id: PeerId, proposal: LeaseProposal },
  ReceivedChallengeRequest { peer_id: PeerId, challenge_key: ChallengeKey },
  ReceivedRetrieveRequest { peer_id: PeerId, nonce: u64 },
}

#[async_trait]
pub trait Service: Stream<Item = Event> + Send + Sync + Clone + Unpin + 'static {
  async fn challenge(&self, peer_id: PeerId, challenge_key: ChallengeKey) -> anyhow::Result<ChallengeProof>;
  async fn send_proposal(
    &self,
    peer_id: PeerId,
    nonce: u64,
    terms: LeaseTerms,
    signature: Signature,
    data: Vec<u8>,
  ) -> String;
  async fn send_challenge_proof(&self, peer_id: PeerId, challenge_key: ChallengeKey, challenge_proof: ChallengeProof);
  async fn send_retrieve_delivery(&self, peer_id: PeerId, nonce: u64, data: Vec<u8>);
  async fn send_proposal_rejection(&self, peer_id: PeerId, nonce: u64, reason: String);
  async fn retrieve(&self, peer_id: PeerId, nonce: u64) -> anyhow::Result<Vec<u8>>;
  fn find_public_key(&self, peer_id: &PeerId) -> Option<secp256k1::PublicKey>;
  fn known_peers(&self) -> Vec<PeerId>;
}

struct TokioExecutor {}

impl Executor for TokioExecutor {
  fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
    tokio::task::spawn(future);
  }
}

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

  Ok(Implementation {
    behaviour: Arc::new(Mutex::new(swarm)),
    pending_challenges: Arc::new(Mutex::new(OneshotListerners::new())),
    pending_retrieves: Arc::new(Mutex::new(OneshotListerners::new())),
    pending_proposals: Arc::new(Mutex::new(OneshotListerners::new())),
  })
}

// TODO pending_* timeouts and cleanup
struct Implementation {
  behaviour: Arc<Mutex<Swarm<behaviour::Behaviour>>>,
  pending_challenges: Arc<Mutex<OneshotListerners<(PeerId, ChallengeKey), ChallengeProof>>>,
  pending_retrieves: Arc<Mutex<OneshotListerners<(PeerId, u64), Vec<u8>>>>,
  pending_proposals: Arc<Mutex<OneshotListerners<(PeerId, u64), String>>>,
}

trait Notify<K, V> {
  fn notify(&self, key: &K, value: V) -> usize;
}

impl<K: std::hash::Hash + std::cmp::Eq, V: Clone> Notify<K, V> for Arc<Mutex<OneshotListerners<K, V>>> {
  fn notify(&self, key: &K, value: V) -> usize {
    self.lock().unwrap().notify(key, value)
  }
}

trait Listeners<K, V> {
  type FutureType: Future<Output = V>;
  fn new_listener(&self, key: K) -> Self::FutureType;
}

impl<K: std::hash::Hash + std::cmp::Eq + 'static, V: Clone + Send + 'static> Listeners<K, V>
  for Arc<Mutex<OneshotListerners<K, V>>>
{
  type FutureType = Box<dyn Future<Output = V> + Send + Sync + Unpin + 'static>;

  fn new_listener(&self, key: K) -> Self::FutureType {
    Box::new(self.lock().unwrap().new_listener(key))
  }
}

impl Clone for Implementation {
  fn clone(&self) -> Self {
    Implementation {
      behaviour: Arc::clone(&self.behaviour),
      pending_challenges: Arc::clone(&self.pending_challenges),
      pending_retrieves: Arc::clone(&self.pending_retrieves),
      pending_proposals: Arc::clone(&self.pending_proposals),
    }
  }
}

impl Stream for Implementation {
  type Item = Event;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    while let Poll::Ready(e) = futures::stream::StreamExt::poll_next_unpin(self.behaviour.lock().unwrap().deref_mut(), cx) {
      match e {
        Some(SwarmEvent::Behaviour(be)) => match be {
          behaviour::Event::ReceivedLeaseProposal { peer_id, proposal } => {
            return Poll::Ready(Some(Event::ReceivedLeaseProposal { peer_id, proposal }));
          }
          behaviour::Event::ReceivedChallengeRequest { peer_id, challenge_key } => {
            return Poll::Ready(Some(Event::ReceivedChallengeRequest { peer_id, challenge_key }));
          }
          behaviour::Event::ReceivedChallengeResponse {
            peer_id,
            challenge_key,
            challenge_proof,
          } => {
            let count = self
              .pending_challenges
              .notify(&(peer_id, challenge_key.clone()), challenge_proof);
            if count == 0 {
              warn!(
                "received a proof not expected peer_id={} nonce={} block_number={}",
                peer_id, challenge_key.nonce, challenge_key.block_number
              );
            }
          }
          behaviour::Event::ReceivedRetrieveRequest { peer_id, nonce } => {
            return Poll::Ready(Some(Event::ReceivedRetrieveRequest { peer_id, nonce }));
          }
          behaviour::Event::ReceivedRetrieveDelivery { peer_id, nonce, data } => {
            let count = self.pending_retrieves.notify(&(peer_id, nonce), data);
            if count == 0 {
              warn!("received retrieve delivery not expected peer_id={} nonce={}", peer_id, nonce);
            }
          }
          behaviour::Event::ReceivedLeaseProposalRejection { peer_id, nonce, reason } => {
            let count = self.pending_proposals.notify(&(peer_id, nonce), reason.clone());
            if count == 0 {
              warn!(
                "received a proposal rejection not expected peer_id={} nonce={} reason={}",
                peer_id, nonce, reason
              );
            }
          }
        },
        Some(other) => {
          trace!("TODO: swarm: {:?}", other);
        }
        None => {
          return Poll::Ready(None);
        }
      }
    }
    Poll::Pending
  }
}

#[async_trait]
impl Service for Implementation {
  async fn challenge(&self, peer_id: PeerId, challenge_key: ChallengeKey) -> anyhow::Result<ChallengeProof> {
    let listener = self.pending_challenges.new_listener((peer_id, challenge_key.clone()));
    self
      .behaviour
      .lock()
      .unwrap()
      .behaviour_mut()
      .p2pim
      .send_challenge(peer_id, challenge_key);
    Ok(listener.await)
  }

  async fn send_proposal(
    &self,
    peer_id: PeerId,
    nonce: u64,
    terms: LeaseTerms,
    signature: Signature,
    data: Vec<u8>,
  ) -> String {
    let listener = self.pending_proposals.new_listener((peer_id, nonce));
    self.behaviour.lock().unwrap().behaviour_mut().p2pim.send_proposal(
      peer_id,
      p2pim::LeaseProposal {
        nonce,
        lease_terms: terms,
        signature,
        data,
      },
    );
    listener.await
  }

  async fn send_challenge_proof(&self, peer_id: PeerId, challenge_key: ChallengeKey, challenge_proof: ChallengeProof) {
    let mut guard = self.behaviour.lock().unwrap();
    guard
      .behaviour_mut()
      .p2pim
      .send_challenge_proof(peer_id, challenge_key, challenge_proof);
  }

  async fn send_retrieve_delivery(&self, peer_id: PeerId, nonce: u64, data: Vec<u8>) {
    let mut guard = self.behaviour.lock().unwrap();
    guard.behaviour_mut().p2pim.send_retrieve_delivery(peer_id, nonce, data);
  }

  async fn send_proposal_rejection(&self, peer_id: PeerId, nonce: u64, reason: String) {
    let mut guard = self.behaviour.lock().unwrap();
    guard.behaviour_mut().p2pim.send_proposal_rejection(peer_id, nonce, reason);
  }

  async fn retrieve(&self, peer_id: PeerId, nonce: u64) -> anyhow::Result<Vec<u8>> {
    let listener = self.pending_retrieves.new_listener((peer_id, nonce));
    self
      .behaviour
      .lock()
      .unwrap()
      .behaviour_mut()
      .p2pim
      .send_retrieve_request(peer_id, nonce);
    let data = listener.await;
    Ok(data)
  }

  fn find_public_key(&self, peer_id: &PeerId) -> Option<PublicKey> {
    let guard = self.behaviour.lock().unwrap();
    guard.behaviour().peer_info(peer_id).and_then(|i| {
      if let libp2p::identity::PublicKey::Secp256k1(p) = i.public_key.clone() {
        Some(p)
      } else {
        None
      }
    })
  }

  fn known_peers(&self) -> Vec<PeerId> {
    let guard = self.behaviour.lock().unwrap();
    guard.behaviour().known_peers()
  }
}
