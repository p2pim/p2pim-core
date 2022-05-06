use web3::signing::keccak256;
use web3::types::Address;

pub fn public_key_to_address(key: &libp2p::identity::secp256k1::PublicKey) -> Address {
  let public_key_bytes = key.encode_uncompressed();
  let hash = keccak256(&public_key_bytes[1..]);
  return Address::from_slice(&hash[12..]);
}
