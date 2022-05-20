use futures::FutureExt;
use log::error;
use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;

pub struct OneshotListerners<K: Hash + Eq, V: Clone> {
  inner: HashMap<K, Vec<futures::channel::oneshot::Sender<V>>>,
}

impl<K: Hash + Eq, V: Clone> OneshotListerners<K, V> {
  pub fn new() -> Self {
    OneshotListerners { inner: HashMap::new() }
  }

  pub fn new_listener(&mut self, key: K) -> impl Future<Output = V> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    self.inner.entry(key).or_default().push(sender);
    receiver.map(|r| r.expect("we never cancel the sender"))
  }

  pub fn notify(&mut self, key: K, value: V) -> usize {
    let senders = self.inner.remove(&key).unwrap_or_else(Vec::new);
    let res = senders.len();
    senders.into_iter().for_each(|sender| {
      if sender.send(value.clone()).is_err() {
        error!("TODO receiver has been dropped, this should not happen");
      }
    });
    res
  }
}
