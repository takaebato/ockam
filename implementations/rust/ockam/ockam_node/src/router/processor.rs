use super::{AddressRecord, NodeState, Router, SenderPair, WorkerMeta};
use crate::{error::NodeError, RouterReason};
use ockam_core::compat::{sync::Arc, vec::Vec};
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{Address, AddressAndMetadata, Error, Result};

impl Router {
    /// Start a processor
    pub(crate) fn start_processor(
        &self,
        addrs: Vec<Address>,
        senders: SenderPair,
        addresses_metadata: Vec<AddressAndMetadata>,
    ) -> Result<()> {
        if self.state.running() {
            self.start_processor_impl(addrs, senders, addresses_metadata)
        } else {
            match self.state.node_state() {
                NodeState::Stopping => Err(Error::new(
                    Origin::Node,
                    Kind::Shutdown,
                    "The node is shutting down",
                ))?,
                NodeState::Running => unreachable!(),
                NodeState::Terminated => unreachable!(),
            }
        }
    }

    fn start_processor_impl(
        &self,
        addrs: Vec<Address>,
        senders: SenderPair,
        addresses_metadata: Vec<AddressAndMetadata>,
    ) -> Result<()> {
        let primary_addr = addrs
            .first()
            .ok_or_else(|| NodeError::RouterState(RouterReason::EmptyAddressSet).internal())?;

        debug!("Starting new processor '{}'", &primary_addr);
        let SenderPair { msgs, ctrl } = senders;

        let record = AddressRecord::new(
            addrs.clone(),
            msgs,
            ctrl,
            // We don't keep track of the mailbox count for processors
            // because, while they are able to send and receive messages
            // via their mailbox, most likely this metric is going to be
            // irrelevant.  We may want to re-visit this decision in the
            // future, if the way processors are used changes.
            Arc::new(0.into()),
            WorkerMeta {
                processor: true,
                detached: false,
            },
        );

        self.map
            .insert_address_record(primary_addr.clone(), record, addresses_metadata)
    }

    /// Stop the processor
    pub(crate) async fn stop_processor(&self, addr: &Address) -> Result<()> {
        trace!("Stopping processor '{}'", addr);

        // Resolve any secondary address to the primary address
        let primary_address = match self.map.get_primary_address(addr) {
            Some(p) => p.clone(),
            None => {
                return Err(Error::new(Origin::Node, Kind::NotFound, "No such address")
                    .context("Address", addr.clone()))
            }
        };

        // Then send processor shutdown signal
        self.map.stop(&primary_address).await?;
        Ok(())
    }
}
