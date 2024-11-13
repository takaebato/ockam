use crate::channel_types::{MessageSender, SmallSender};
use crate::error::{NodeError, NodeReason};
use crate::relay::CtrlSignal;
use alloc::string::String;
use core::default::Default;
use core::fmt::Debug;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use ockam_core::compat::sync::Mutex as SyncMutex;
use ockam_core::compat::sync::RwLock as SyncRwLock;
use ockam_core::compat::sync::RwLockWriteGuard as SyncRwLockWriteGuard;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{
    compat::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
        vec::Vec,
    },
    flow_control::FlowControls,
    Address, AddressAndMetadata, AddressMetadata, Error, RelayMessage, Result,
};

#[derive(Default)]
struct AddressMaps {
    /// Registry of primary address to worker address record state
    address_records_map: BTreeMap<Address, AddressRecord>,
    /// Alias-registry to map arbitrary address to primary addresses
    alias_map: BTreeMap<Address, Address>,
    /// Registry of arbitrary metadata for each address, lazily populated
    address_metadata_map: BTreeMap<Address, AddressMetadata>,
}

/// Address states and associated logic
pub struct InternalMap {
    address_maps: SyncRwLock<AddressMaps>,
    /// The order in which clusters are allocated and de-allocated
    cluster_order: SyncMutex<Vec<String>>,
    /// Cluster data records
    clusters: SyncMutex<BTreeMap<String, BTreeSet<Address>>>,
    /// Track stop information for Clusters
    stopping: SyncMutex<BTreeSet<Address>>,
    /// Access to [`FlowControls`] to clean resources
    flow_controls: FlowControls,
    /// Metrics collection and sharing
    #[cfg(feature = "metrics")]
    metrics: (Arc<AtomicUsize>, Arc<AtomicUsize>),
}
impl InternalMap {
    pub(crate) fn resolve(&self, addr: &Address) -> Result<MessageSender<RelayMessage>> {
        let guard = self.address_maps.read().unwrap();

        let address_record = if let Some(primary_address) = guard.alias_map.get(addr) {
            guard.address_records_map.get(primary_address)
        } else {
            trace!("Resolving worker address '{addr}'... FAILED; no such worker");
            return Err(Error::new(Origin::Node, Kind::NotFound, "No such address")
                .context("Address", addr.clone()));
        };

        match address_record {
            Some(address_record) => {
                if let Some(sender) = address_record.sender() {
                    trace!("Resolving worker address '{addr}'... OK");
                    address_record.increment_msg_count();
                    Ok(sender)
                } else {
                    trace!("Resolving worker address '{addr}'... REJECTED; worker shutting down");
                    Err(
                        Error::new(Origin::Node, Kind::Shutdown, "Worker shutting down")
                            .context("Address", addr.clone()),
                    )
                }
            }
            None => {
                trace!("Resolving worker address '{addr}'... FAILED; no such worker");
                Err(Error::new(Origin::Node, Kind::NotFound, "No such address")
                    .context("Address", addr.clone()))
            }
        }
    }
}

impl InternalMap {
    pub(super) fn new(flow_controls: &FlowControls) -> Self {
        Self {
            address_maps: Default::default(),
            cluster_order: SyncMutex::new(Default::default()),
            clusters: SyncMutex::new(Default::default()),
            stopping: SyncMutex::new(Default::default()),
            flow_controls: flow_controls.clone(),
            #[cfg(feature = "metrics")]
            metrics: Default::default(),
        }
    }
}

impl InternalMap {
    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    pub(super) fn clear_address_records_map(&self) {
        self.address_maps
            .write()
            .unwrap()
            .address_records_map
            .clear()
    }

    pub(super) async fn stop(&self, primary_address: &Address) -> Result<()> {
        {
            let mut guard = self.stopping.lock().unwrap();
            if guard.contains(primary_address) {
                return Ok(());
            } else {
                guard.insert(primary_address.clone());
            }
        }

        let send_signal_future = {
            let guard = self.address_maps.read().unwrap();
            if let Some(record) = guard.address_records_map.get(primary_address) {
                record.stop()
            } else {
                return Err(Error::new(Origin::Node, Kind::NotFound, "No such address")
                    .context("Address", primary_address.clone()));
            }
        };

        // we can't call `.await` while holding the lock
        if let Some(send_signal_future) = send_signal_future {
            send_signal_future.await
        } else {
            Ok(())
        }
    }

    pub(super) fn is_worker_registered_at(&self, primary_address: &Address) -> bool {
        self.address_maps
            .read()
            .unwrap()
            .address_records_map
            .contains_key(primary_address)
        // TODO: we should also check aliases
    }

    pub(super) fn list_workers(&self) -> Vec<Address> {
        self.address_maps
            .read()
            .unwrap()
            .address_records_map
            .keys()
            .cloned()
            .collect()
    }

    pub(super) fn insert_address_record(
        &self,
        primary_address: Address,
        record: AddressRecord,
        addresses_metadata: Vec<AddressAndMetadata>,
    ) -> Result<()> {
        let mut guard = self.address_maps.write().unwrap();

        // check if the address already exists
        if let Some(record) = guard.address_records_map.get(&primary_address) {
            if record.check_integrity() {
                let node = NodeError::Address(primary_address.clone());
                return Err(node.already_exists());
            } else {
                self.free_address_impl(&mut guard, &primary_address);
            }
        }

        // check if aliases are already in use
        for addr in record.address_set() {
            if guard.alias_map.contains_key(addr) {
                let node = NodeError::Address(primary_address.clone());
                return Err(node.already_exists());
            }
        }

        record.address_set.iter().for_each(|addr| {
            guard
                .alias_map
                .insert(addr.clone(), primary_address.clone());
        });

        for metadata in addresses_metadata {
            if !record.address_set().contains(&metadata.address) {
                warn!(
                    "Address {} is not in the set of addresses",
                    metadata.address
                );
                continue;
            }

            let entry = guard
                .address_metadata_map
                .entry(metadata.address)
                .or_default();
            *entry = metadata.metadata;
        }

        _ = guard.address_records_map.insert(primary_address, record);
        Ok(())
    }

    pub(super) fn find_terminal_address(
        &self,
        addresses: &[Address],
    ) -> Option<AddressAndMetadata> {
        addresses.iter().find_map(|address| {
            self.address_maps
                .read()
                .unwrap()
                .address_metadata_map
                .get(address)
                .filter(|&meta| meta.is_terminal)
                .map(|meta| AddressAndMetadata {
                    address: address.clone(),
                    metadata: meta.clone(),
                })
        })
    }

    pub(super) fn get_address_metadata(&self, address: &Address) -> Option<AddressMetadata> {
        self.address_maps
            .read()
            .unwrap()
            .address_metadata_map
            .get(address)
            .cloned()
    }

    pub(super) fn get_primary_address(&self, alias_address: &Address) -> Option<Address> {
        self.address_maps
            .read()
            .unwrap()
            .alias_map
            .get(alias_address)
            .cloned()
    }
}

impl InternalMap {
    #[cfg(feature = "metrics")]
    pub(super) fn update_metrics(&self) {
        self.metrics.0.store(
            self.address_maps.read().unwrap().address_records_map.len(),
            Ordering::Release,
        );
        self.metrics
            .1
            .store(self.clusters.lock().unwrap().len(), Ordering::Release);
    }

    #[cfg(feature = "metrics")]
    pub(super) fn get_metrics(&self) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
        (Arc::clone(&self.metrics.0), Arc::clone(&self.metrics.1))
    }

    #[cfg(feature = "metrics")]
    pub(super) fn get_addr_count(&self) -> usize {
        self.metrics.0.load(Ordering::Acquire)
    }

    /// Add an address to a particular cluster
    pub(super) fn set_cluster(&self, label: String, primary: Address) -> Result<()> {
        let address_records_guard = self.address_maps.read().unwrap();
        let rec = address_records_guard
            .address_records_map
            .get(&primary)
            .ok_or_else(|| NodeError::Address(primary).not_found())?;

        let mut clusters_guard = self.clusters.lock().unwrap();

        // If this is the first time we see this cluster ID
        if !clusters_guard.contains_key(&label) {
            clusters_guard.insert(label.clone(), BTreeSet::new());
            self.cluster_order.lock().unwrap().push(label.clone());
        }

        // Add all addresses to the cluster set
        for addr in rec.address_set() {
            clusters_guard
                .get_mut(&label)
                .expect("No such cluster??")
                .insert(addr.clone());
        }

        Ok(())
    }

    /// Retrieve the next cluster in reverse-initialisation order
    /// Return None if there is no next cluster or if the cluster
    /// contained no more active addresses
    pub(super) fn next_cluster(&self) -> Option<Vec<Address>> {
        // loop until either:
        //  - there are no more clusters
        //  - we found a non-empty list of active addresses in a cluster
        loop {
            let name = self.cluster_order.lock().unwrap().pop()?;
            let addrs = self.clusters.lock().unwrap().remove(&name)?;
            let active_addresses: Vec<Address> = self
                .address_maps
                .read()
                .unwrap()
                .address_records_map
                .iter()
                .filter_map(|(primary, _)| {
                    if addrs.contains(primary) {
                        Some(primary.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if active_addresses.is_empty() {
                continue;
            } else {
                return Some(active_addresses);
            }
        }
    }

    /// Mark this address as "having started to stop"
    pub(super) fn init_stop(&self, addr: Address) {
        self.stopping.lock().unwrap().insert(addr);
    }

    /// Check whether the current cluster of addresses was stopped
    pub(super) fn cluster_done(&self) -> bool {
        self.stopping.lock().unwrap().is_empty()
    }

    /// Stop all workers not in a cluster, returns their primary addresses
    pub(super) async fn stop_all_non_cluster_workers(&self) -> Vec<Address> {
        let mut futures = Vec::new();
        let mut addresses = Vec::new();
        {
            let clustered =
                self.clusters
                    .lock()
                    .unwrap()
                    .iter()
                    .fold(BTreeSet::new(), |mut acc, (_, set)| {
                        acc.append(&mut set.clone());
                        acc
                    });

            let guard = self.address_maps.read().unwrap();
            let records: Vec<&AddressRecord> = guard
                .address_records_map
                .iter()
                .filter_map(|(addr, rec)| {
                    if clustered.contains(addr) {
                        None
                    } else {
                        Some(rec)
                    }
                })
                // Filter all detached workers because they don't matter :(
                .filter(|rec| !rec.meta.detached)
                .collect();

            for record in records {
                if let Some(first_address) = record.address_set.first() {
                    debug!("Stopping address {}", first_address);
                    addresses.push(first_address.clone());
                    let send_stop_signal_future = record.stop();
                    if let Some(send_stop_signal_future) = send_stop_signal_future {
                        futures.push((first_address.clone(), send_stop_signal_future));
                    }
                } else {
                    error!("Empty Address Set during graceful shutdown")
                }
            }
        }

        // We can only call `.await` outside the lock
        for (first_address, send_stop_signal_future) in futures {
            send_stop_signal_future.await.unwrap_or_else(|e| {
                error!("Failed to stop address {}: {}", first_address, e);
            });
        }

        addresses
    }

    /// Permanently free all remaining resources associated to a particular address
    pub(super) fn free_address(&self, primary: &Address) {
        let mut guard = self.address_maps.write().unwrap();
        self.free_address_impl(&mut guard, primary);
    }

    fn free_address_impl(&self, guard: &mut SyncRwLockWriteGuard<AddressMaps>, primary: &Address) {
        self.stopping.lock().unwrap().remove(primary);
        self.flow_controls.cleanup_address(primary);

        let removed = guard.address_records_map.remove(primary);
        if let Some(record) = removed {
            for alias_address in record.address_set {
                guard.address_metadata_map.remove(&alias_address);
                guard.alias_map.remove(&alias_address);
            }
        }
    }
}

/// Additional metadata for worker records
#[derive(Debug)]
pub struct WorkerMeta {
    pub processor: bool,
    pub detached: bool,
}

pub struct AddressRecord {
    address_set: Vec<Address>,
    sender: SyncMutex<Option<MessageSender<RelayMessage>>>,
    ctrl_tx: SmallSender<CtrlSignal>, // Unused for not-detached workers
    state: AtomicU8,
    meta: WorkerMeta,
    msg_count: Arc<AtomicUsize>,
}

impl Debug for AddressRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AddressRecord")
            .field("address_set", &self.address_set)
            .field("sender", &self.sender.lock().unwrap().is_some())
            .field("ctrl_tx", &self.ctrl_tx)
            .field("state", &self.state)
            .field("meta", &self.meta)
            .field("msg_count", &self.msg_count)
            .finish()
    }
}

impl AddressRecord {
    pub fn address_set(&self) -> &[Address] {
        &self.address_set
    }

    pub fn sender(&self) -> Option<MessageSender<RelayMessage>> {
        self.sender.lock().unwrap().clone()
    }

    pub fn drop_sender(&self) {
        *self.sender.lock().unwrap() = None;
    }

    pub fn new(
        address_set: Vec<Address>,
        sender: MessageSender<RelayMessage>,
        ctrl_tx: SmallSender<CtrlSignal>,
        msg_count: Arc<AtomicUsize>,
        meta: WorkerMeta,
    ) -> Self {
        AddressRecord {
            address_set,
            sender: SyncMutex::new(Some(sender)),
            ctrl_tx,
            state: AtomicU8::new(AddressState::Running as u8),
            msg_count,
            meta,
        }
    }

    #[inline]
    pub fn increment_msg_count(&self) {
        self.msg_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Signal this worker to stop -- it will no longer be able to receive messages
    /// Since the worker is held behind a lock, we cannot hold the synchronous lock
    /// and call an `.await` to send the signal.
    /// To avoid this shortcoming, we return a future that can be awaited on instead.
    pub fn stop(&self) -> Option<impl core::future::Future<Output = Result<()>>> {
        if self.state.load(Ordering::Relaxed) != AddressState::Running as u8 {
            return None;
        }

        // the stop can be triggered only once
        let result = self.state.compare_exchange(
            AddressState::Running as u8,
            AddressState::Stopping as u8,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        if result.is_err() {
            return None;
        }

        if self.meta.processor {
            let ctrl_tx = self.ctrl_tx.clone();
            Some(async move {
                ctrl_tx
                    .send(CtrlSignal::InterruptStop)
                    .await
                    .map_err(|_| NodeError::NodeState(NodeReason::Unknown).internal())?;
                Ok(())
            })
        } else {
            self.drop_sender();
            None
        }
    }

    /// Check the integrity of this record
    #[inline]
    pub fn check_integrity(&self) -> bool {
        self.state.load(Ordering::Relaxed) == AddressState::Running as u8
            && self.sender.lock().unwrap().is_some()
    }
}

/// Encode the run states a worker or processor can be in
#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum AddressState {
    /// The runner is looping in its main body (either handling messages or a manual run-loop)
    Running,
    /// The runner was signaled to shut-down (running `shutdown()`)
    Stopping,
    /// The runner has experienced an error and is waiting for supervisor intervention
    #[allow(unused)]
    Faulty,
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::channel_types::small_channel;
    use crate::router::record::InternalMap;

    #[test]
    fn test_next_cluster() {
        let map = InternalMap::new(&FlowControls::new());

        // create 3 clusters
        //   cluster 1 has one active address
        //   cluster 2 has no active address
        //   cluster 3 has one active address
        map.address_maps
            .write()
            .unwrap()
            .address_records_map
            .insert("address1".into(), create_address_record("address1"));
        let _ = map.set_cluster("CLUSTER1".into(), "address1".into());

        map.address_maps
            .write()
            .unwrap()
            .address_records_map
            .insert("address2".into(), create_address_record("address2"));
        let _ = map.set_cluster("CLUSTER2".into(), "address2".into());
        map.free_address(&"address2".into());

        map.address_maps
            .write()
            .unwrap()
            .address_records_map
            .insert("address3".into(), create_address_record("address3"));
        let _ = map.set_cluster("CLUSTER3".into(), "address3".into());

        // get the active addresses for cluster 3, clusters are popped in reverse order
        assert_eq!(map.next_cluster(), Some(vec!["address3".into()]));
        // get the active addresses for cluster 1, cluster 2 is skipped
        assert_eq!(map.next_cluster(), Some(vec!["address1".into()]));
        // finally there are no more clusters with active addresses
        assert_eq!(map.next_cluster(), None);
    }

    /// HELPERS
    fn create_address_record(primary: &str) -> AddressRecord {
        let (tx1, _) = small_channel();
        let (tx2, _) = small_channel();
        AddressRecord::new(
            vec![primary.into()],
            tx1,
            tx2,
            Arc::new(AtomicUsize::new(1)),
            WorkerMeta {
                processor: false,
                detached: false,
            },
        )
    }
}
