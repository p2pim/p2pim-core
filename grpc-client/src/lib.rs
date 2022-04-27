include!(concat!(env!("OUT_DIR"), "/p2pim.api.rs"));

#[cfg(test)]
mod tests {
  #[test]
  fn it_works() {
    assert_eq!(2 + 2, 4);
  }
}
