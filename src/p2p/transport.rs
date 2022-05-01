use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::transport::Boxed;
use libp2p::core::upgrade::{SelectUpgrade, Version};
use libp2p::dns::TokioDnsConfig;
use libp2p::mplex::MplexConfig;
use libp2p::noise::NoiseConfig;
use libp2p::tcp::TokioTcpConfig;
use libp2p::yamux::YamuxConfig;
use libp2p::{identity, noise, PeerId, Transport};
use std::io;
use std::io::{Error, ErrorKind};
use std::time::Duration;

pub type TTransport = Boxed<(PeerId, StreamMuxerBox)>;

pub fn build_transport(keypair: identity::Keypair) -> io::Result<TTransport> {
  let xx_keypair = noise::Keypair::<noise::X25519Spec>::new().into_authentic(&keypair).unwrap();
  let noise_config = NoiseConfig::xx(xx_keypair).into_authenticated();

  Ok(
    TokioDnsConfig::system(TokioTcpConfig::new())?
      .upgrade(Version::V1)
      .authenticate(noise_config)
      .multiplex(SelectUpgrade::new(YamuxConfig::default(), MplexConfig::new()))
      .timeout(Duration::from_secs(20))
      .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
      .map_err(|err| Error::new(ErrorKind::Other, err))
      .boxed(),
  )
}
