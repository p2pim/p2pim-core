use crate::libp2p::protobuf;
use crate::libp2p::protobuf::handler;
use crate::proto;
use crate::proto::p2p::protocol_message::Message;
use crate::proto::p2p::{protocol_message, ChallengeRequest, ChallengeResponse, RetrieveDelivery, RetrieveRequest};
use crate::types::{ChallengeKey, ChallengeProof, LeaseTerms, Signature};
use libp2p::core::connection::ConnectionId;
use libp2p::core::ConnectedPoint;
use libp2p::swarm::{
  ConnectionHandler, IntoConnectionHandler, NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters,
};
use libp2p::{Multiaddr, PeerId};
use log::{trace, warn};
use std::collections::VecDeque;
use std::convert::{TryFrom, TryInto};
use std::task::{Context, Poll, Waker};
use web3::types::H256;

const P2PIM_PROTOCOL_NAME: &[u8] = b"/p2pim/protobuf/0.1.0";

pub struct Behaviour {
  message_queue: VecDeque<(PeerId, protocol_message::Message)>,
  event_queue: VecDeque<Event>,
  waker: Option<Waker>,
}

impl Default for Behaviour {
  fn default() -> Self {
    Behaviour::new()
  }
}

impl Behaviour {
  pub fn new() -> Self {
    Behaviour {
      message_queue: VecDeque::new(),
      event_queue: VecDeque::new(),
      waker: None,
    }
  }

  pub fn send_proposal(&mut self, peer_id: PeerId, lease_proposal: LeaseProposal) {
    self
      .message_queue
      .push_back((peer_id, Message::LeaseProposal(lease_proposal.into())));
    self.wake()
  }

  pub fn send_challenge(&mut self, peer_id: PeerId, challenge_key: ChallengeKey) {
    self.message_queue.push_back((
      peer_id,
      Message::ChallengeRequest(ChallengeRequest {
        nonce: challenge_key.nonce,
        block_number: challenge_key.block_number,
      }),
    ));
    self.wake()
  }

  pub fn send_challenge_proof(&mut self, peer_id: PeerId, challenge_key: ChallengeKey, challenge_proof: ChallengeProof) {
    self.message_queue.push_back((
      peer_id,
      Message::ChallengeResponse(ChallengeResponse {
        nonce: challenge_key.nonce,
        block_number: challenge_key.block_number,
        block_data: challenge_proof.block_data,
        proof: challenge_proof.proof.into_iter().map(|p| H256(p).into()).collect(),
      }),
    ));
    self.wake()
  }

  pub fn send_retrieve_request(&mut self, peer_id: PeerId, nonce: u64) {
    self
      .message_queue
      .push_back((peer_id, Message::RetrieveRequest(RetrieveRequest { nonce })));
    self.wake()
  }

  pub fn send_retrieve_delivery(&mut self, peer_id: PeerId, nonce: u64, data: Vec<u8>) {
    self
      .message_queue
      .push_back((peer_id, Message::RetrieveDelivery(RetrieveDelivery { nonce, data })));
    self.wake()
  }

  fn wake(&mut self) {
    if let Some(waker) = self.waker.take() {
      waker.wake();
    }
  }
}

#[derive(Debug)]
pub enum Event {
  ReceivedLeaseProposal(PeerId, LeaseProposal),
  ReceivedChallengeRequest(PeerId, ChallengeKey),
  ReceivedChallengeResponse(PeerId, ChallengeKey, ChallengeProof),
  ReceivedRetrieveRequest(PeerId, u64),
  ReceivedRetrieveDelivery(PeerId, u64, Vec<u8>),
}

#[derive(Debug)]
pub struct LeaseProposal {
  pub nonce: u64,
  pub lease_terms: LeaseTerms,
  pub signature: Signature,
  pub data: Vec<u8>,
}

impl TryFrom<proto::p2p::LeaseProposal> for LeaseProposal {
  type Error = String;

  fn try_from(value: proto::p2p::LeaseProposal) -> Result<Self, Self::Error> {
    let lease_terms = value.lease_terms.as_ref().ok_or("lease_terms empty")?;
    Ok(LeaseProposal {
      nonce: value.nonce,
      lease_terms: LeaseTerms {
        token_address: lease_terms.token_address.as_ref().ok_or("token_address empty")?.into(),
        price: lease_terms.price.as_ref().ok_or("price empty")?.into(),
        penalty: lease_terms.penalty.as_ref().ok_or("penalty empty")?.into(),
        proposal_expiration: lease_terms
          .proposal_expiration
          .clone()
          .ok_or("proposal_expiration empty")?
          .try_into()
          .map_err(|e| format!("{}", e))?,
        lease_duration: lease_terms
          .lease_duration
          .clone()
          .ok_or("lease_duration empty")?
          .try_into()
          .map_err(|_| "lease_duration should be positive")?,
      },
      signature: Signature::deserialize(value.signature.as_slice()).map_err(|e| format!("{}", e))?,
      data: value.data,
    })
  }
}

impl From<LeaseProposal> for proto::p2p::LeaseProposal {
  fn from(value: LeaseProposal) -> Self {
    let lease_terms = &value.lease_terms;
    proto::p2p::LeaseProposal {
      nonce: value.nonce,
      lease_terms: Some(proto::p2p::lease_proposal::LeaseTerms {
        token_address: Some((&lease_terms.token_address).into()),
        price: Some((&lease_terms.price).into()),
        penalty: Some((&lease_terms.penalty).into()),
        proposal_expiration: Some(lease_terms.proposal_expiration.into()),
        lease_duration: Some(lease_terms.lease_duration.into()),
      }),
      signature: value.signature.serialize(),
      data: value.data,
    }
  }
}

#[derive(Debug)]
pub enum SourceData {
  Data(Vec<u8>),
}

impl NetworkBehaviour for Behaviour {
  type ConnectionHandler = protobuf::handler::Handler<proto::p2p::ProtocolMessage>;
  type OutEvent = Event;

  fn new_handler(&mut self) -> Self::ConnectionHandler {
    protobuf::handler::Handler::new(P2PIM_PROTOCOL_NAME)
  }

  fn inject_connection_established(
    &mut self,
    _peer_id: &PeerId,
    _connection_id: &ConnectionId,
    _endpoint: &ConnectedPoint,
    _failed_addresses: Option<&Vec<Multiaddr>>,
    _other_established: usize,
  ) {
  }

  fn inject_connection_closed(
    &mut self,
    _peer_id: &PeerId,
    _connection_id: &ConnectionId,
    _endpoint: &ConnectedPoint,
    _handler: <Self::ConnectionHandler as IntoConnectionHandler>::Handler,
    _remaining_established: usize,
  ) {
  }

  fn inject_event(
    &mut self,
    peer_id: PeerId,
    _: ConnectionId,
    event: <<Self::ConnectionHandler as IntoConnectionHandler>::Handler as ConnectionHandler>::OutEvent,
  ) {
    match event {
      handler::Event::MessageReceived(message) => match message.message {
        Some(Message::ChallengeRequest(challenge_request)) => self.event_queue.push_back(Event::ReceivedChallengeRequest(
          peer_id,
          ChallengeKey {
            nonce: challenge_request.nonce,
            block_number: challenge_request.block_number,
          },
        )),
        Some(Message::ChallengeResponse(challenge_response)) => self.event_queue.push_back(Event::ReceivedChallengeResponse(
          peer_id,
          ChallengeKey {
            nonce: challenge_response.nonce,
            block_number: challenge_response.block_number,
          },
          ChallengeProof {
            block_data: challenge_response.block_data,
            proof: challenge_response.proof.into_iter().map(|h| H256::from(&h).0).collect(),
          },
        )),
        Some(Message::LeaseProposal(lease_proposal)) => {
          match lease_proposal.try_into().map(|p| Event::ReceivedLeaseProposal(peer_id, p)) {
            Err(e) => warn!("invalid lease proposal received: {}", e),
            Ok(p) => self.event_queue.push_back(p),
          }
        }
        Some(Message::LeaseRejection(_)) => todo!("handle lease rejection"),
        Some(Message::RetrieveRequest(retrieve_request)) => self
          .event_queue
          .push_back(Event::ReceivedRetrieveRequest(peer_id, retrieve_request.nonce)),
        Some(Message::RetrieveDelivery(retrieve_delivery)) => self.event_queue.push_back(Event::ReceivedRetrieveDelivery(
          peer_id,
          retrieve_delivery.nonce,
          retrieve_delivery.data,
        )),
        None => warn!("invalid message received from peer {}: no inner message", peer_id),
      },
    };
  }

  fn poll(
    &mut self,
    cx: &mut Context<'_>,
    _: &mut impl PollParameters,
  ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler>> {
    let ready_send_message = |peer_id, message| {
      Poll::Ready(NetworkBehaviourAction::NotifyHandler {
        peer_id,
        event: proto::p2p::ProtocolMessage { message: Some(message) },
        handler: NotifyHandler::Any,
      })
    };

    if let Some((peer_id, message)) = self.message_queue.pop_front() {
      return ready_send_message(peer_id, message);
    }

    if let Some(event) = self.event_queue.pop_front() {
      return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
    }

    if let Some(waker) = self.waker.as_ref() {
      if !cx.waker().will_wake(waker) {
        self.waker = Some(cx.waker().clone());
      }
    } else {
      self.waker = Some(cx.waker().clone());
    }

    Poll::Pending
  }
}
