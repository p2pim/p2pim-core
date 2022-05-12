use asynchronous_codec::{BytesMut, Decoder, Encoder, Framed, LengthCodec};
use futures::future;
use libp2p::core::UpgradeInfo;
use libp2p::swarm::NegotiatedSubstream;
use libp2p::{InboundUpgrade, OutboundUpgrade};
use std::marker::PhantomData;
use std::{io, iter};
use void::Void;

pub struct Protocol<T: prost::Message> {
  protocol_name: Vec<u8>,
  phantom: PhantomData<T>,
}

impl<T: prost::Message> Protocol<T> {
  pub fn new(name: &[u8]) -> Self {
    Protocol {
      protocol_name: name.to_vec(),
      phantom: PhantomData,
    }
  }
}

impl<T: prost::Message> UpgradeInfo for Protocol<T> {
  type Info = Vec<u8>;
  type InfoIter = iter::Once<Self::Info>;

  fn protocol_info(&self) -> Self::InfoIter {
    iter::once(self.protocol_name.clone())
  }
}

pub struct ProtobufDelimitedCodec<T: prost::Message> {
  inner: LengthCodec,
  phantom_data: PhantomData<T>,
}

impl<T: prost::Message> Default for ProtobufDelimitedCodec<T> {
  fn default() -> Self {
    ProtobufDelimitedCodec {
      inner: LengthCodec,
      phantom_data: PhantomData,
    }
  }
}

impl<T: prost::Message + Default> Decoder for ProtobufDelimitedCodec<T> {
  type Item = T;
  type Error = DecodeError;

  fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
    match self.inner.decode(src) {
      Ok(None) => Ok(None),
      Ok(Some(b)) => T::decode(b).map(Some).map_err(DecodeError::DecodeError),
      Err(e) => Err(DecodeError::IOError(e)),
    }
  }
}

impl<T: prost::Message + Default> Encoder for ProtobufDelimitedCodec<T> {
  type Item = T;
  type Error = io::Error;

  fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
    let mut bytes = BytesMut::with_capacity(item.encoded_len());
    item
      .encode(&mut bytes)
      .expect("this cannot happen as the only error is not enough buffer");
    self.inner.encode(bytes.into(), dst)
  }
}

#[derive(Debug)]
pub enum DecodeError {
  IOError(io::Error),
  DecodeError(prost::DecodeError),
}

impl From<io::Error> for DecodeError {
  fn from(e: io::Error) -> Self {
    DecodeError::IOError(e)
  }
}

pub type ProtocolType<T> = Framed<NegotiatedSubstream, ProtobufDelimitedCodec<T>>;

impl<T: prost::Message + Default> InboundUpgrade<NegotiatedSubstream> for Protocol<T> {
  type Output = ProtocolType<T>;
  type Error = Void;
  type Future = future::Ready<Result<Self::Output, Self::Error>>;

  fn upgrade_inbound(self, stream: NegotiatedSubstream, _: Self::Info) -> Self::Future {
    // FIXME The ProtobufDelimitedCode does not have a limit on message size, could eventually explode
    future::ok(Framed::new(stream, ProtobufDelimitedCodec::default()))
  }
}

impl<T: prost::Message + Default> OutboundUpgrade<NegotiatedSubstream> for Protocol<T> {
  type Output = ProtocolType<T>;
  type Error = Void;
  type Future = future::Ready<Result<Self::Output, Self::Error>>;

  fn upgrade_outbound(self, stream: NegotiatedSubstream, _: Self::Info) -> Self::Future {
    future::ok(Framed::new(stream, ProtobufDelimitedCodec::default()))
  }
}
