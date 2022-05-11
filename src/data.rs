use crate::types::DataParameters;
use crate::utils::merkle::MerkleTree;
use tonic::async_trait;

#[async_trait]
pub trait Service: Send + Sync + Unpin + Clone + 'static {
  async fn parameters(&self, data: &[u8]) -> DataParameters;
}

#[derive(Clone)]
struct Implementation;

pub fn new_service() -> impl Service {
  Implementation
}

#[async_trait]
impl Service for Implementation {
  async fn parameters(&self, data: &[u8]) -> DataParameters {
    let mut merkle = MerkleTree::new();
    merkle.update(data);
    let merkle_root = merkle.finalize();
    DataParameters {
      merkle_root,
      size: data.len(),
    }
  }
}
