fn main() -> Result<(), Box<dyn std::error::Error>> {
  tonic_build::compile_protos("proto/solidity.proto")?;
  tonic_build::compile_protos("proto/api.proto")?;
  prost_build::compile_protos(&["proto/p2p.proto"], &["proto/"])?;
  Ok(())
}
