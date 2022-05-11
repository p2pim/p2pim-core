pub mod proto {
  pub mod api {
    tonic::include_proto!("api");
  }
  pub mod multiformats {
    include!(concat!(env!("OUT_DIR"), "/multiformats.rs"));
  }
  pub mod p2p {
    include!(concat!(env!("OUT_DIR"), "/p2p.rs"));
  }
  pub mod libp2p {
    use libp2p::multihash;
    use std::convert::{TryFrom, TryInto};
    tonic::include_proto!("libp2p");

    impl From<&libp2p::PeerId> for PeerId {
      fn from(value: &libp2p::PeerId) -> Self {
        PeerId { data: value.to_bytes() }
      }
    }

    impl From<libp2p::PeerId> for PeerId {
      fn from(value: libp2p::PeerId) -> Self {
        From::from(&value)
      }
    }

    impl TryFrom<&PeerId> for libp2p::PeerId {
      type Error = multihash::Error;

      fn try_from(value: &PeerId) -> Result<Self, Self::Error> {
        libp2p::PeerId::from_bytes(value.data.as_slice())
      }
    }

    impl TryFrom<PeerId> for libp2p::PeerId {
      type Error = multihash::Error;

      fn try_from(value: PeerId) -> Result<Self, Self::Error> {
        (&value).try_into()
      }
    }
  }
  pub mod solidity {
    use num_bigint::{BigInt, Sign};
    use std::convert::TryFrom;
    tonic::include_proto!("solidity");

    impl From<&Address> for web3::types::Address {
      fn from(proto_address: &Address) -> Self {
        // TODO Tis function can panic, check others as well
        web3::types::Address::from_slice(proto_address.data.as_slice())
      }
    }

    impl From<&web3::types::Address> for Address {
      fn from(web3_address: &web3::types::Address) -> Self {
        Address {
          data: web3_address.as_bytes().to_vec(),
        }
      }
    }

    impl From<web3::types::Address> for Address {
      fn from(web3_address: web3::types::Address) -> Self {
        From::from(&web3_address)
      }
    }

    impl From<&Uint256> for web3::types::U256 {
      fn from(proto_u256: &Uint256) -> Self {
        web3::types::U256::from_little_endian(proto_u256.data_le.as_slice())
      }
    }

    impl From<&Uint256> for num_bigint::BigInt {
      fn from(proto_u256: &Uint256) -> Self {
        num_bigint::BigInt::from_bytes_le(num_bigint::Sign::Plus, proto_u256.data_le.as_slice())
      }
    }

    impl TryFrom<BigInt> for Uint256 {
      type Error = String;

      fn try_from(value: BigInt) -> Result<Self, Self::Error> {
        if value.sign() == Sign::Minus {
          Err("negative number".to_string())
        } else {
          let (_, bytes) = value.to_bytes_le();
          Ok(Uint256 { data_le: bytes })
        }
      }
    }

    impl From<&web3::types::U256> for Uint256 {
      fn from(web3_u256: &web3::types::U256) -> Self {
        let mut available_u8: [u8; 4 * 8] = Default::default();
        web3_u256.to_little_endian(&mut available_u8);
        Uint256 {
          data_le: available_u8.to_vec(),
        }
      }
    }

    impl From<web3::types::U256> for Uint256 {
      fn from(web3_u256: web3::types::U256) -> Self {
        From::from(&web3_u256)
      }
    }

    impl From<&H256> for web3::types::H256 {
      fn from(proto_h256: &H256) -> Self {
        web3::types::H256::from_slice(proto_h256.data.as_slice())
      }
    }

    impl From<&web3::types::H256> for H256 {
      fn from(web3_h256: &web3::types::H256) -> Self {
        H256 {
          data: web3_h256.as_bytes().to_vec(),
        }
      }
    }

    impl From<web3::types::H256> for H256 {
      fn from(web3_h256: web3::types::H256) -> Self {
        From::from(&web3_h256)
      }
    }
  }
}

pub mod daemon;
pub mod data;
pub mod grpc;
pub mod libp2p;
pub mod onchain;
pub mod p2p;
pub mod persistence;
pub mod reactor;
pub mod s3;
pub mod types;
pub mod utils;
