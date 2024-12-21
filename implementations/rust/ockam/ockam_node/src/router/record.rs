use crate::channel_types::{MessageSender, OneshotSender};
use crate::error::{NodeError, NodeReason};
use crate::relay::CtrlSignal;
use alloc::string::String;
use core::default::Default;
use core::fmt::Debug;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use ockam_core::compat::collections::hash_map::EntryRef;
use ockam_core::compat::sync::RwLock as SyncRwLock;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{
    compat::{
        collections::{HashMap, HashSet},
        sync::Arc,
        vec::Vec,
    },
    flow_control::FlowControls,
    Address, AddressAndMetadata, AddressMetadata, Error, RelayMessage, Result,
};

#[derive(Default)]
struct AddressMaps {
    // NOTE: It's crucial that if more that one of these structures is needed to perform an
    // operation, we should always acquire locks in the order they're declared here. Otherwise, it
    // can cause a deadlock.
    /// Registry of primary address to worker address record state
    records: SyncRwLock<HashMap<Address, AddressRecord>>,
    /// Alias-registry to map arbitrary address to primary addresses
    aliases: SyncRwLock<HashMap<Address, Address>>,
    /// Registry of arbitrary metadata for each address, lazily populated
    metadata: SyncRwLock<HashMap<Address, AddressMetadata>>,
}

#[derive(Default)]
struct ClusterMaps {
    /// The order in which clusters are allocated and de-allocated
    order: Vec<String>,
    /// Key is Label
    map: HashMap<String, HashSet<Address>>, // Only primary addresses here
}

/// Address states and associated logic
pub struct InternalMap {
    // NOTE: It's crucial that if more that one of these structures is needed to perform an
    // operation, we should always acquire locks in the order they're declared here. Otherwise, it
    // can cause a deadlock.
    address_maps: AddressMaps,
    cluster_maps: SyncRwLock<ClusterMaps>,
    /// Track addresses that are being stopped atm
    stopping: SyncRwLock<HashSet<Address>>,
    /// Access to [`FlowControls`] to clean resources
    flow_controls: FlowControls,
    /// Metrics collection and sharing
    #[cfg(feature = "metrics")]
    metrics: (Arc<AtomicUsize>, Arc<AtomicUsize>),
}

impl InternalMap {
    pub(crate) fn resolve(&self, addr: &Address) -> Result<MessageSender<RelayMessage>> {
        let records = self.address_maps.records.read().unwrap();
        let aliases = self.address_maps.aliases.read().unwrap();

        let address_record = if let Some(primary_address) = aliases.get(addr) {
            records.get(primary_address)
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
            cluster_maps: Default::default(),
            stopping: Default::default(),
            flow_controls: flow_controls.clone(),
            #[cfg(feature = "metrics")]
            metrics: Default::default(),
        }
    }
}

impl InternalMap {
    pub(super) fn stop(&self, address: &Address) -> Result<()> {
        // To guarantee consistency we'll first acquire lock on all the maps we need to touch
        // and only then start modifications
        let mut records = self.address_maps.records.write().unwrap();
        let mut aliases = self.address_maps.aliases.write().unwrap();
        let mut metadata = self.address_maps.metadata.write().unwrap();
        let mut stopping = self.stopping.write().unwrap();

        let primary_address = aliases
            .get(address)
            .ok_or_else(|| {
                Error::new(Origin::Node, Kind::NotFound, "No such address")
                    .context("Address", address.clone())
            })?
            .clone();

        self.flow_controls.cleanup_address(&primary_address);

        let record = if let Some(record) = records.remove(&primary_address) {
            record
        } else {
            return Err(Error::new(Origin::Node, Kind::NotFound, "No such address")
                .context("Address", address.clone()));
        };

        for address in &record.address_set {
            metadata.remove(address);
            aliases.remove(address);
        }

        stopping.insert(primary_address);

        record.stop()?;

        Ok(())
    }

    pub(super) fn stop_ack(&self, primary_address: &Address) -> bool {
        let mut stopping = self.stopping.write().unwrap();
        stopping.remove(primary_address);

        stopping.is_empty()
    }

    pub(super) fn is_worker_registered_at(&self, primary_address: &Address) -> bool {
        self.address_maps
            .records
            .read()
            .unwrap()
            .contains_key(primary_address)
        // TODO: we should also check aliases
    }

    pub(super) fn list_workers(&self) -> Vec<Address> {
        self.address_maps
            .records
            .read()
            .unwrap()
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
        let mut records = self.address_maps.records.write().unwrap();

        let entry = records.entry_ref(&primary_address);

        // FIXME: removed integrity check

        let entry = match entry {
            EntryRef::Occupied(_) => {
                let node = NodeError::Address(primary_address);
                return Err(node.already_exists());
            }
            EntryRef::Vacant(entry) => entry,
        };

        {
            // check if aliases are already in use
            let mut aliases = self.address_maps.aliases.write().unwrap();
            for addr in record.address_set() {
                if aliases.contains_key(addr) {
                    let node = NodeError::Address(primary_address.clone());
                    return Err(node.already_exists());
                }
            }

            for addr in record.address_set() {
                aliases.insert(addr.clone(), primary_address.clone());
            }
        }

        if !addresses_metadata.is_empty() {
            let mut metadata = self.address_maps.metadata.write().unwrap();
            for address_metadata in addresses_metadata {
                if !record.address_set().contains(&address_metadata.address) {
                    warn!(
                        "Address {} is not in the set of addresses",
                        address_metadata.address
                    );
                    continue;
                }

                metadata.insert(address_metadata.address, address_metadata.metadata);
            }
        }

        entry.insert(record);

        Ok(())
    }

    pub(super) fn find_terminal_address(
        &self,
        addresses: &[Address],
    ) -> Option<AddressAndMetadata> {
        addresses.iter().find_map(|address| {
            self.address_maps
                .metadata
                .read()
                .unwrap()
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
            .metadata
            .read()
            .unwrap()
            .get(address)
            .cloned()
    }
}

impl InternalMap {
    #[cfg(feature = "metrics")]
    pub(super) fn update_metrics(&self) {
        self.metrics.0.store(
            self.address_maps.records.read().unwrap().len(),
            Ordering::Release,
        );
        self.metrics.1.store(
            self.cluster_maps.read().unwrap().map.len(),
            Ordering::Release,
        );
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
    pub(super) fn set_cluster(&self, primary: &Address, label: String) -> Result<()> {
        if !self
            .address_maps
            .records
            .read()
            .unwrap()
            .contains_key(primary)
        {
            return Err(NodeError::Address(primary.clone()).not_found())?;
        }

        // If this is the first time we see this cluster ID
        let mut cluster_maps = self.cluster_maps.write().unwrap();
        match cluster_maps.map.entry_ref(&label) {
            EntryRef::Occupied(mut occupied_entry) => {
                occupied_entry.get_mut().insert(primary.clone());
            }
            EntryRef::Vacant(vacant_entry) => {
                vacant_entry.insert_entry(HashSet::from([primary.clone()]));
                cluster_maps.order.push(label.clone());
            }
        }

        Ok(())
    }

    /// Stop all workers not in a cluster, returns their primary addresses
    pub(super) fn stop_all_non_cluster_workers(&self) -> bool {
        // All clustered addresses
        let clustered = self.cluster_maps.read().unwrap().map.iter().fold(
            HashSet::new(),
            |mut acc, (_, set)| {
                acc.extend(set.iter().cloned());
                acc
            },
        );

        // HashMap::extract_if would fit nicely, but it's a nightly-only API, unless we use
        // hashbrown directly
        let records_to_stop: Vec<AddressRecord> = {
            let mut records = self.address_maps.records.write().unwrap();
            let addresses_to_stop: HashSet<Address> = records
                .iter()
                .filter_map(|(addr, rec)| {
                    // Filter all detached or clustered workers
                    if !clustered.contains(addr) && !rec.meta.detached {
                        Some(addr.clone())
                    } else {
                        None
                    }
                })
                .collect();

            let mut records_to_stop = Vec::with_capacity(addresses_to_stop.len());
            for address in &addresses_to_stop {
                let record = records.remove(address).unwrap();
                records_to_stop.push(record);
            }

            records_to_stop
        };

        let was_empty = records_to_stop.is_empty();

        for record in records_to_stop {
            if let Err(err) = record.stop() {
                error!("Error stopping address. Err={}", err);
                continue;
            }
        }

        was_empty
    }

    pub(super) fn stop_all_cluster_workers(&self) -> bool {
        let mut cluster_maps = self.cluster_maps.write().unwrap();
        let mut records = self.address_maps.records.write().unwrap();

        let mut was_empty = true;
        while let Some(label) = cluster_maps.order.pop() {
            if let Some(addrs) = cluster_maps.map.remove(&label) {
                for addr in addrs {
                    match records.remove(&addr) {
                        Some(record) => {
                            was_empty = false;
                            match record.stop() {
                                Ok(_) => {}
                                Err(err) => {
                                    error!(
                                        "Error stopping address {} from cluster {}. Err={}",
                                        addr, label, err
                                    );
                                }
                            }
                        }
                        None => {
                            warn!(
                                "Stopping address {} from cluster {} but it doesn't exist",
                                addr, label
                            );
                        }
                    }
                }
            }
        }

        was_empty
    }

    pub(super) fn force_clear_records(&self) -> Vec<Address> {
        let mut records = self.address_maps.records.write().unwrap();

        records.drain().map(|(address, _record)| address).collect()
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
    sender: Option<MessageSender<RelayMessage>>,
    ctrl_tx: OneshotSender<CtrlSignal>, // Unused for not-detached workers
    state: AtomicU8,
    meta: WorkerMeta,
    msg_count: Arc<AtomicUsize>,
}

impl Debug for AddressRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AddressRecord")
            .field("address_set", &self.address_set)
            .field("sender", &self.sender.is_some())
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
        self.sender.clone()
    }

    pub fn drop_sender(&mut self) {
        self.sender = None;
    }

    pub fn new(
        address_set: Vec<Address>,
        sender: MessageSender<RelayMessage>,
        ctrl_tx: OneshotSender<CtrlSignal>,
        msg_count: Arc<AtomicUsize>,
        meta: WorkerMeta,
    ) -> Self {
        AddressRecord {
            address_set,
            sender: Some(sender),
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
    pub fn stop(mut self) -> Result<()> {
        if self.state.load(Ordering::Relaxed) != AddressState::Running as u8 {
            return Ok(());
        }

        // the stop can be triggered only once
        let result = self.state.compare_exchange(
            AddressState::Running as u8,
            AddressState::Stopping as u8,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        if result.is_err() {
            return Ok(());
        }

        if self.meta.processor {
            self.ctrl_tx
                .send(CtrlSignal::InterruptStop)
                .map_err(|_| NodeError::NodeState(NodeReason::Unknown).internal())?;
        } else {
            // TODO: Recheck
            if !self.meta.detached {
                self.drop_sender();
            }
        }

        Ok(())
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
