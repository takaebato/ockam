use crate::router::{AddressRecord, NodeState, Router, SenderPair, WorkerMeta};
use crate::{error::NodeError, RouterReason};
use core::sync::atomic::AtomicUsize;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{
    compat::{sync::Arc, vec::Vec},
    Address, AddressAndMetadata, Error, Result,
};

impl Router {
    /// Start a new worker
    pub fn start_worker(
        &self,
        addrs: Vec<Address>,
        senders: SenderPair,
        detached: bool,
        addresses_metadata: Vec<AddressAndMetadata>,
        metrics: Arc<AtomicUsize>,
    ) -> Result<()> {
        if !self.state.running() {
            match self.state.node_state() {
                NodeState::Stopping => Err(Error::new(
                    Origin::Node,
                    Kind::Shutdown,
                    "The node is shutting down",
                ))?,
                NodeState::Running => unreachable!(),
                NodeState::Terminated => unreachable!(),
            }
        } else {
            self.start_worker_impl(addrs, senders, detached, addresses_metadata, metrics)
        }
    }

    fn start_worker_impl(
        &self,
        addrs: Vec<Address>,
        senders: SenderPair,
        detached: bool,
        addresses_metadata: Vec<AddressAndMetadata>,
        metrics: Arc<AtomicUsize>,
    ) -> Result<()> {
        let primary_addr = addrs
            .first()
            .ok_or_else(|| NodeError::RouterState(RouterReason::EmptyAddressSet).internal())?;

        debug!("Starting new worker '{}'", primary_addr);
        let SenderPair { msgs, ctrl } = senders;

        // Create an address record and insert it into the internal map
        let address_record = AddressRecord::new(
            addrs.clone(),
            msgs,
            ctrl,
            metrics,
            WorkerMeta {
                processor: false,
                detached,
            },
        );

        self.map
            .insert_address_record(primary_addr.clone(), address_record, addresses_metadata)
    }

    /// Stop the worker
    pub async fn stop_worker(&self, addr: &Address, detached: bool) -> Result<()> {
        debug!("Stopping worker '{}'", addr);

        // Resolve any secondary address to the primary address
        let primary_address = match self.map.get_primary_address(addr) {
            Some(p) => p,
            None => {
                return Err(Error::new(Origin::Node, Kind::NotFound, "No such address")
                    .context("Address", addr))
            }
        };

        // If we are dropping a real worker, then we simply close the
        // mailbox channel to trigger a graceful worker self-shutdown.
        // The worker shutdown will call `free_address()`.
        //
        // For detached workers (i.e. Context's without a mailbox relay
        // running) we simply free the address.
        if detached {
            self.map.free_address(&primary_address);
        } else {
            self.map.stop(&primary_address).await?;
        }

        Ok(())
    }
}
