use log::info;
use std::error::Error;
use std::net::SocketAddr;
use warp::path::Tail;
use warp::{reject, Filter};

pub async fn listen_and_serve(s3_addr: SocketAddr) -> Result<(), Box<dyn Error>> {
  info!("starting S3 compatible server on {}", s3_addr);
  let put_object = warp::put().and(warp::path::tail()).and_then(|tail: Tail| async move {
    if tail.as_str().is_empty() {
      Err(reject()) // TODO this is a 404, should be a 400 or 403
    } else {
      Ok(format!("TODO: PutObject with key {}, not implemented", tail.as_str()))
    }
  });
  warp::serve(put_object).run(s3_addr).await;
  Ok(())
}
