use super::p2pim;
use super::p2pim::LeaseProposal;
use libp2p::identify::{Identify, IdentifyConfig, IdentifyEvent, IdentifyInfo};
use libp2p::identity::PublicKey;
use libp2p::mdns::{Mdns, MdnsConfig, MdnsEvent};
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourAction, NetworkBehaviourEventProcess};
use libp2p::{ping, NetworkBehaviour, PeerId};
use log::{debug, info, trace, warn};
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::task::Poll;

const PROTOCOL_VERSION: &str = "p2pim/0.1.0";

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true, poll_method = "poll", out_event = "Event")]
pub struct Behaviour {
  identify: Identify,
  ping: ping::Behaviour,
  mdns: Mdns,
  pub p2pim: p2pim::Behaviour,
  #[behaviour(ignore)]
  actions: VecDeque<BehaviourAction>,
  #[behaviour(ignore)]
  known_peers: HashMap<PeerId, IdentifyInfo>,
  #[behaviour(ignore)]
  events_queue: VecDeque<Event>,
}

#[derive(Debug)]
pub enum Event {
  ReceivedLeaseProposal { peer_id: PeerId, proposal: LeaseProposal },
}

#[derive(Debug)]
enum BehaviourAction {
  Dial(PeerId),
}

impl Behaviour {
  pub async fn new(local_public_key: PublicKey) -> Result<Self, Box<dyn Error>> {
    let identify = Identify::new(
      IdentifyConfig::new(PROTOCOL_VERSION.to_string(), local_public_key).with_agent_version("p2pim-core".to_string()),
    );
    let ping = ping::Behaviour::new(ping::Config::new().with_keep_alive(true)); // TODO This is temporary until we maintain the connection in p2pim
    let mdns = Mdns::new(MdnsConfig::default()).await?;
    let p2pim = p2pim::Behaviour::new();
    Ok(Behaviour {
      identify,
      ping,
      mdns,
      p2pim,
      actions: VecDeque::new(),
      known_peers: HashMap::new(),
      events_queue: VecDeque::new(),
    })
  }

  pub fn peer_info(&self, peer_id: &PeerId) -> Option<&IdentifyInfo> {
    self.known_peers.get(peer_id)
  }

  pub fn known_peers(&self) -> Vec<PeerId> {
    // TODO copying the peers in memory
    self.known_peers.keys().map(Clone::clone).collect()
  }

  fn poll(
    &mut self,
    _: &mut std::task::Context,
    _: &mut impl libp2p::swarm::PollParameters,
  ) -> std::task::Poll<
    libp2p::swarm::NetworkBehaviourAction<
      <Self as NetworkBehaviour>::OutEvent,
      <Self as NetworkBehaviour>::ConnectionHandler,
    >,
  > {
    if let Some(action) = self.actions.pop_front() {
      match action {
        BehaviourAction::Dial(peer_id) => {
          return Poll::Ready(NetworkBehaviourAction::Dial {
            handler: self.new_handler(),
            opts: DialOpts::peer_id(peer_id).condition(PeerCondition::Disconnected).build(),
          })
        }
      }
    }

    if let Some(event) = self.events_queue.pop_front() {
      return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
    }
    Poll::Pending
  }
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for Behaviour {
  fn inject_event(&mut self, event: IdentifyEvent) {
    trace!("identify: event received: {:?}", event);
    match event {
      IdentifyEvent::Received { peer_id, info } => {
        if info.protocol_version != PROTOCOL_VERSION {
          debug!("received peer info for incompatible protocol: {}", info.protocol_version);
        } else {
          let peer_id_from_public = PeerId::from_public_key(&info.public_key);
          if peer_id_from_public != peer_id {
            warn!("peer sending wrong public key peer_id={}", peer_id);
          } else {
            if let libp2p::identity::PublicKey::Secp256k1(_) = info.public_key.clone() {
              info!("known peer with id {}: {:?}", peer_id, info);
              self.known_peers.insert(peer_id, info);
            } else {
              warn!("peer sending a public key not supported: {:?}", info.public_key);
            }
          }
        }
      }
      _ => trace!("ignored identify event"),
    }
  }
}

impl NetworkBehaviourEventProcess<ping::Event> for Behaviour {
  fn inject_event(&mut self, event: ping::Event) {
    trace!("ping: event received: {:?}", event);
  }
}

impl NetworkBehaviourEventProcess<p2pim::Event> for Behaviour {
  fn inject_event(&mut self, event: p2pim::Event) {
    trace!("p2pim: event received: {:?}", event);
    match event {
      p2pim::Event::ReceivedLeaseProposal(peer_id, proposal) => self
        .events_queue
        .push_back(Event::ReceivedLeaseProposal { peer_id, proposal }),
    }
  }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for Behaviour {
  fn inject_event(&mut self, event: MdnsEvent) {
    trace!("mdns: event received: {:?}", event);
    match event {
      MdnsEvent::Discovered(addr_iter) => {
        addr_iter.for_each(|(peer_id, _)| self.actions.push_back(BehaviourAction::Dial(peer_id)))
      }
      MdnsEvent::Expired(_) => debug!("mdns: expired event ignored, nothing to do"),
    }
  }
}
