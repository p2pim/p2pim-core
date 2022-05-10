use web3::signing::keccak256;
use web3::types::Address;

pub trait IntoAddress {
  fn into_address(self) -> Address;
}

impl IntoAddress for &libp2p::identity::secp256k1::PublicKey {
  fn into_address(self) -> Address {
    let public_key = self.encode_uncompressed();
    as_address(public_key)
  }
}

impl IntoAddress for &secp256k1::PublicKey {
  fn into_address(self) -> Address {
    let public_key = self.serialize_uncompressed();
    as_address(public_key)
  }
}

fn as_address(raw: [u8; 65]) -> Address {
  debug_assert_eq!(raw[0], 0x04);
  let hash = keccak256(&raw[1..]);
  return Address::from_slice(&hash[12..]);
}
