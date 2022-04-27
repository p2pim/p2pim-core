fn main() -> Result<(), Box<dyn std::error::Error>> {
  tonic_build::compile_protos("proto/solidity.proto")?;
  tonic_build::compile_protos("proto/api.proto")?;
  Ok(())
}
