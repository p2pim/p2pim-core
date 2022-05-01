use libp2p::identify::{Identify, IdentifyConfig, IdentifyEvent};
use libp2p::identity::PublicKey;
use libp2p::mdns::{Mdns, MdnsConfig, MdnsEvent};
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourAction, NetworkBehaviourEventProcess};
use libp2p::{ping, NetworkBehaviour, PeerId};
use log::{debug, trace};
use std::collections::VecDeque;
use std::error::Error;
use std::task::Poll;

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true, poll_method = "poll")]
pub struct Behaviour {
  identify: Identify,
  ping: ping::Behaviour,
  mdns: Mdns,
  #[behaviour(ignore)]
  actions: VecDeque<BehaviourAction>,
}

#[derive(Debug)]
enum BehaviourAction {
  Dial(PeerId),
}

impl Behaviour {
  pub async fn new(local_public_key: PublicKey) -> Result<Self, Box<dyn Error>> {
    let identify = Identify::new(
      IdentifyConfig::new("p2pim/0.1.0".to_string(), local_public_key).with_agent_version("p2pim-core".to_string()),
    );
    let ping = ping::Behaviour::new(ping::Config::new().with_keep_alive(true)); // TODO This is temporary until we maintain the connection in p2pim
    let mdns = Mdns::new(MdnsConfig::default()).await?;
    Ok(Behaviour {
      identify,
      ping,
      mdns,
      actions: VecDeque::new(),
    })
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
        BehaviourAction::Dial(peer_id) => Poll::Ready(NetworkBehaviourAction::Dial {
          handler: self.new_handler(),
          opts: DialOpts::peer_id(peer_id).condition(PeerCondition::Disconnected).build(),
        }),
      }
    } else {
      Poll::Pending
    }
  }
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for Behaviour {
  fn inject_event(&mut self, event: IdentifyEvent) {
    trace!("event from identify: {:?}", event);
  }
}

impl NetworkBehaviourEventProcess<ping::Event> for Behaviour {
  fn inject_event(&mut self, event: ping::Event) {
    trace!("event from ping: {:?}", event);
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
