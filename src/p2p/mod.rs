use libp2p::core::Executor;
use libp2p::identity::Keypair;
use libp2p::swarm::{NetworkBehaviour, SwarmBuilder};
use libp2p::{PeerId, Swarm};
use std::borrow::Borrow;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;

pub mod behaviour;
pub mod transport;

pub async fn create_swarm(
  keypair: Keypair,
) -> Result<Swarm<impl NetworkBehaviour<OutEvent = impl std::fmt::Debug>>, Box<dyn Error>> {
  let transport = transport::build_transport(keypair.clone())?;
  let behaviour = behaviour::Behaviour::new(keypair.public()).await?;
  let local_peer_id = PeerId::from_public_key(keypair.public().borrow());
  let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
    .executor(Box::new(TokioExecutor {}))
    .build();
  swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

  Ok(swarm)
}

struct TokioExecutor {}

impl Executor for TokioExecutor {
  fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
    tokio::task::spawn(future);
  }
}
