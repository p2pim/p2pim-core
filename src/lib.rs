pub mod proto {
  pub mod api {
    tonic::include_proto!("api");
  }
  pub mod solidity {
    tonic::include_proto!("solidity");

    impl From<&Address> for web3::types::Address {
      fn from(proto_address: &Address) -> Self {
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

    impl From<&Uint256> for web3::types::U256 {
      fn from(proto_u256: &Uint256) -> Self {
        web3::types::U256::from_little_endian(proto_u256.data_le.as_slice())
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
  }
}

pub mod daemon;
