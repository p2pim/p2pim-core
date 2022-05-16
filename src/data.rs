use crate::cryptography;
use crate::cryptography::MerkleTree;
use crate::types::DataParameters;
use anyhow::{ensure, Context};
use libp2p::PeerId;
use std::path::PathBuf;
use tonic::async_trait;

#[async_trait]
pub trait Service: Send + Sync + Unpin + Clone + 'static {
  async fn parameters(&self, data: &[u8]) -> DataParameters;
  async fn store(&self, peer_id: PeerId, nonce: u64, data: &[u8]) -> anyhow::Result<DataParameters>;
  async fn proof(&self, peer_id: PeerId, nonce: u64, block_number: usize) -> anyhow::Result<(Vec<u8>, Vec<[u8; 32]>)>;
  async fn verify(&self, params: DataParameters, block_number: u32, block_data: &[u8], proof: Vec<[u8; 32]>) -> bool;
}

#[derive(Clone)]
struct Implementation<TCryptography>
where
  TCryptography: cryptography::Service,
{
  _cryptography: TCryptography,
  data_folder: PathBuf,
}

pub fn new_service<TCryptography>(cryptography: TCryptography, data_folder: PathBuf) -> impl Service
where
  TCryptography: cryptography::Service,
{
  Implementation {
    _cryptography: cryptography,
    data_folder,
  }
}

#[async_trait]
impl<TCryptography> Service for Implementation<TCryptography>
where
  TCryptography: cryptography::Service,
{
  async fn parameters(&self, data: &[u8]) -> DataParameters {
    let mut merkle = TCryptography::new_merkle_tree();
    merkle.append_data(data);
    let merkle_root = merkle.root();
    DataParameters {
      merkle_root: merkle_root.to_vec(),
      size: data.len(),
    }
  }

  async fn store(&self, peer_id: PeerId, nonce: u64, data: &[u8]) -> anyhow::Result<DataParameters> {
    let parameters = self.parameters(data).await;
    let mut peer_id_path = self.data_folder.clone();
    peer_id_path.push(peer_id.to_base58());
    tokio::fs::create_dir_all(peer_id_path.clone())
      .await
      .context("error storing data from peer")?;
    peer_id_path.push(nonce.to_string());
    tokio::fs::write(peer_id_path, data)
      .await
      .context("error storing data from peer")?;
    Ok(parameters)
  }

  async fn proof(&self, peer_id: PeerId, nonce: u64, block_number: usize) -> anyhow::Result<(Vec<u8>, Vec<[u8; 32]>)> {
    let mut peer_id_path = self.data_folder.clone();
    peer_id_path.push(peer_id.to_base58());
    peer_id_path.push(nonce.to_string());

    let data: Vec<u8> = tokio::fs::read(peer_id_path.clone())
      .await
      .with_context(|| format!("Failed to read file while creating proof file={:?}", peer_id_path))?;

    let block_start: usize = (block_number as usize) * cryptography::BLOCK_SIZE_BYTES;
    ensure!(data.len() >= block_start, "block is out of bounds");
    let block_end = std::cmp::min(block_start + cryptography::BLOCK_SIZE_BYTES, data.len());
    let block_data = data[block_start..block_end].to_vec();

    let mut merkle = TCryptography::new_merkle_tree();
    merkle.append_data(data);
    Ok((block_data, merkle.proof(block_number)))
  }

  async fn verify(&self, params: DataParameters, block_number: u32, block_data: &[u8], proof: Vec<[u8; 32]>) -> bool {
    let mut merkle_root: [u8; 32] = Default::default();
    merkle_root.copy_from_slice(params.merkle_root.as_slice());
    TCryptography::verify(block_number as usize, block_data, proof, merkle_root, params.size)
  }
}
