use super::protocol::{DecodeError, ProtocolType};
use futures::{SinkExt, StreamExt};
use libp2p::swarm::handler::{InboundUpgradeSend, OutboundUpgradeSend};
use libp2p::swarm::{ConnectionHandler, ConnectionHandlerEvent, ConnectionHandlerUpgrErr, KeepAlive, SubstreamProtocol};
use log::{trace, warn};
use std::collections::VecDeque;
use std::fmt::{Debug, Display, Formatter};
use std::io;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use super::protocol;

const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Config {
  idle_timeout: Duration,
  protocol_name: Vec<u8>,
}

pub struct Handler<T: prost::Message> {
  config: Config,
  keep_alive: KeepAlive,
  pending_messages: VecDeque<T>,
  connection: Option<ProtocolType<T>>,
  requested: bool,
}

impl<T: prost::Message> Handler<T> {
  pub fn new(protocol_name: &[u8]) -> Self {
    Handler {
      config: Config {
        idle_timeout: DEFAULT_IDLE_TIMEOUT,
        protocol_name: protocol_name.to_vec(),
      },
      keep_alive: KeepAlive::No,
      pending_messages: VecDeque::new(),
      connection: None,
      requested: false,
    }
  }
}

#[derive(Debug)]
pub enum Event<T: prost::Message> {
  MessageReceived(T),
}

impl<T: prost::Message> Handler<T> {
  fn update_keep_alive(&mut self) {
    self.keep_alive = KeepAlive::Until(Instant::now() + self.config.idle_timeout);
  }
}

#[derive(Debug)]
pub enum HandlerError {
  InboundClosed,
  DecodeError(DecodeError),
  IOError(io::Error),
}

impl Display for HandlerError {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    write!(f, "error TODO: {:?}", self)
  }
}

impl std::error::Error for HandlerError {}

impl<T: prost::Message + Default + 'static> ConnectionHandler for Handler<T> {
  type InEvent = T;
  type OutEvent = Event<T>;
  type Error = HandlerError;
  type InboundProtocol = protocol::Protocol<T>;
  type OutboundProtocol = protocol::Protocol<T>;
  type InboundOpenInfo = ();
  type OutboundOpenInfo = ();

  fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
    SubstreamProtocol::new(protocol::Protocol::new(self.config.protocol_name.as_slice()), ())
  }

  fn inject_fully_negotiated_inbound(
    &mut self,
    protocol: <Self::InboundProtocol as InboundUpgradeSend>::Output,
    _: Self::InboundOpenInfo,
  ) {
    trace!("fully negotiated inbound");
    self.connection = Some(protocol);
    self.update_keep_alive();
  }

  fn inject_fully_negotiated_outbound(
    &mut self,
    protocol: <Self::OutboundProtocol as OutboundUpgradeSend>::Output,
    _: Self::OutboundOpenInfo,
  ) {
    trace!("fully negotiated outbond");
    self.connection = Some(protocol);
    self.update_keep_alive();
  }

  fn inject_event(&mut self, event: Self::InEvent) {
    self.pending_messages.push_back(event);
    self.update_keep_alive();
  }

  fn inject_dial_upgrade_error(
    &mut self,
    _: Self::OutboundOpenInfo,
    error: ConnectionHandlerUpgrErr<<Self::OutboundProtocol as OutboundUpgradeSend>::Error>,
  ) {
    warn!("dial upgrade error: {:?}", error);
  }

  fn connection_keep_alive(&self) -> KeepAlive {
    self.keep_alive
  }

  fn poll(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::OutEvent, Self::Error>> {
    // TODO Refactor this method
    if !self.pending_messages.is_empty() && self.connection.is_none() && !self.requested {
      self.requested = true;
      return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
        protocol: SubstreamProtocol::new(protocol::Protocol::new(self.config.protocol_name.as_slice()), ()),
      });
    }

    let messages = if let Some(out) = self.connection.as_mut() {
      let mut count = 0;
      while let Some(message) = self.pending_messages.pop_front() {
        if let Poll::Ready(state) = out.poll_ready_unpin(cx) {
          match state {
            Ok(()) => {
              trace!("sending message {:?}", message);
              out.start_send_unpin(message).expect("TODO");
              count += 1;
            }
            Err(e) => {
              warn!("outbound is in error estate: {:?}", e);
              return Poll::Ready(ConnectionHandlerEvent::Close(HandlerError::IOError(e)));
            }
          }
        } else {
          warn!("outbound not ready");
          // FIXME very bad code
          break;
        }
      }
      count
    } else {
      0
    };
    if messages > 0 {
      self.update_keep_alive();
    }

    if let Some(inbound) = self.connection.as_mut() {
      match inbound.poll_next_unpin(cx) {
        Poll::Ready(None) => {
          return Poll::Ready(ConnectionHandlerEvent::Close(HandlerError::InboundClosed));
        }
        Poll::Ready(Some(Ok(message))) => {
          trace!("message received: {:?}", message);
          self.update_keep_alive();
          return Poll::Ready(ConnectionHandlerEvent::Custom(Event::MessageReceived(message)));
        }
        Poll::Ready(Some(Err(e))) => {
          return Poll::Ready(ConnectionHandlerEvent::Close(HandlerError::DecodeError(e)));
        }
        Poll::Pending => {}
      }
    }

    if let Some(out) = self.connection.as_mut() {
      match out.poll_flush_unpin(cx) {
        Poll::Ready(Ok(())) => {}
        Poll::Ready(Err(e)) => warn!("error sending message: {}", e),
        Poll::Pending => {}
      }
    }
    Poll::Pending
  }
}
