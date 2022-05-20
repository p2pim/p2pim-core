use futures::{AsyncRead, AsyncReadExt, AsyncWriteExt, FutureExt};
use libp2p::core::connection::ConnectionId;
use libp2p::core::{ConnectedPoint, UpgradeInfo};
use libp2p::swarm::handler::{InboundUpgradeSend, OutboundUpgradeSend};
use libp2p::swarm::{
  ConnectionHandler, ConnectionHandlerEvent, ConnectionHandlerUpgrErr, IntoConnectionHandler, KeepAlive, NegotiatedSubstream,
  NetworkBehaviour, NetworkBehaviourAction, PollParameters, SubstreamProtocol,
};
use libp2p::{InboundUpgrade, Multiaddr, OutboundUpgrade, PeerId};
use std::convert::{TryFrom, TryInto};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{future, io, iter};

const TRANSFER_PROTOCOL_NAME: &[u8] = b"/p2pim/transfer/0.1.0";

pub struct Behaviour {}

impl Default for Behaviour {
  fn default() -> Self {
    Behaviour::new()
  }
}

impl Behaviour {
  pub fn new() -> Self {
    Behaviour {}
  }
}

#[derive(Debug)]
pub enum Event {}

impl NetworkBehaviour for Behaviour {
  type ConnectionHandler = Handler;
  type OutEvent = Event;

  fn new_handler(&mut self) -> Self::ConnectionHandler {
    todo!()
  }

  fn inject_connection_established(
    &mut self,
    _peer_id: &PeerId,
    _connection_id: &ConnectionId,
    _endpoint: &ConnectedPoint,
    _failed_addresses: Option<&Vec<Multiaddr>>,
    _other_established: usize,
  ) {
    todo!()
  }

  fn inject_connection_closed(
    &mut self,
    _peer_id: &PeerId,
    _connection_id: &ConnectionId,
    _endpoint: &ConnectedPoint,
    _handler: <Self::ConnectionHandler as IntoConnectionHandler>::Handler,
    _remaining_established: usize,
  ) {
    todo!()
  }

  fn inject_event(
    &mut self,
    peer_id: PeerId,
    _: ConnectionId,
    event: <<Self::ConnectionHandler as IntoConnectionHandler>::Handler as ConnectionHandler>::OutEvent,
  ) {
    todo!()
  }

  fn poll(
    &mut self,
    cx: &mut Context<'_>,
    _: &mut impl PollParameters,
  ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler>> {
    todo!()
  }
}

pub struct Handler;

impl ConnectionHandler for Handler {
  type InEvent = ();
  type OutEvent = ();
  type Error = io::Error;
  type InboundProtocol = Protocol<Inbound>;
  type OutboundProtocol = Protocol<Outbound>;
  type InboundOpenInfo = ();
  type OutboundOpenInfo = ();

  fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
    SubstreamProtocol::new(Protocol::inbound(), ())
  }

  fn inject_fully_negotiated_inbound(
    &mut self,
    protocol: <Self::InboundProtocol as InboundUpgradeSend>::Output,
    info: Self::InboundOpenInfo,
  ) {
    let pepe = protocol;
  }

  fn inject_fully_negotiated_outbound(
    &mut self,
    protocol: <Self::OutboundProtocol as OutboundUpgradeSend>::Output,
    info: Self::OutboundOpenInfo,
  ) {
    todo!()
  }

  fn inject_event(&mut self, event: Self::InEvent) {
    todo!()
  }

  fn inject_dial_upgrade_error(
    &mut self,
    info: Self::OutboundOpenInfo,
    error: ConnectionHandlerUpgrErr<<Self::OutboundProtocol as OutboundUpgradeSend>::Error>,
  ) {
    todo!()
  }

  fn connection_keep_alive(&self) -> KeepAlive {
    todo!()
  }

  fn poll(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::OutEvent, Self::Error>> {
    todo!()
  }
}

pub struct Protocol<T>(T);

pub struct Inbound();
pub struct Outbound(u64);

impl Protocol<Inbound> {
  fn inbound() -> Self {
    Protocol(Inbound())
  }
}

impl Protocol<Outbound> {
  fn outbound(nonce: u64) -> Self {
    Protocol(Outbound(nonce))
  }
}

impl<T> UpgradeInfo for Protocol<T> {
  type Info = &'static [u8];
  type InfoIter = iter::Once<Self::Info>;

  fn protocol_info(&self) -> Self::InfoIter {
    iter::once(TRANSFER_PROTOCOL_NAME)
  }
}

impl InboundUpgrade<NegotiatedSubstream> for Protocol<Inbound> {
  type Output = (u64, NegotiatedSubstream); // TODO Refine negotiated substream, should be only write
  type Error = io::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;

  fn upgrade_inbound(self, socket: NegotiatedSubstream, _: Self::Info) -> Self::Future {
    read_inbound(socket).boxed()
  }
}

impl OutboundUpgrade<NegotiatedSubstream> for Protocol<Outbound> {
  type Output = NegotiatedSubstream; // TODO Regine negotieated substream, only for read
  type Error = io::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;

  fn upgrade_outbound(self, socket: NegotiatedSubstream, _: Self::Info) -> Self::Future {
    send_outbound(socket, self.0 .0).boxed()
  }
}

async fn read_inbound(mut socket: NegotiatedSubstream) -> Result<(u64, NegotiatedSubstream), io::Error> {
  let mut bytes = [0u8; 8];
  socket.read_exact(&mut bytes).await?;
  let nonce = u64::from_be_bytes(bytes);
  Ok((nonce, socket))
}

async fn send_outbound(mut socket: NegotiatedSubstream, nonce: u64) -> Result<NegotiatedSubstream, io::Error> {
  socket.write_all(nonce.to_be_bytes().as_ref()).await?;
  Ok(socket)
}
