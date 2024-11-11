use super::Router;
use crate::channel_types::MessageSender;
use crate::error::{NodeError, NodeReason};
use crate::router::record::InternalMapSharedState;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{Address, Error, RelayMessage, Result, TransportType};

/// Receive an address and resolve it to a sender
///
/// This function only applies to local address types, and will
/// fail to resolve a correct address if it given a remote
/// address.
pub(crate) fn resolve(
    map: &InternalMapSharedState,
    addr: Address,
) -> Result<(Address, MessageSender<RelayMessage>)> {
    let base = format!("Resolving worker address '{}'...", addr);

    let address_record = if let Some(primary_address) = map.get_primary_address(&addr) {
        map.get_address_sender(&primary_address)
    } else {
        trace!("{} FAILED; no such worker", base);

        return Err(Error::new(Origin::Api, Kind::Other, ""));
    };

    match address_record {
        Some(record) => {
            trace!("{} OK", base);
            Ok((addr, record))
        }
        None => {
            trace!("{} FAILED; no such worker", base);
            Err(Error::new(Origin::Api, Kind::Other, ""))
        }
    }
}

pub(super) fn router_addr(router: &mut Router, tt: TransportType) -> Result<Address> {
    router
        .external
        .get(&tt)
        .cloned()
        .ok_or_else(|| NodeError::NodeState(NodeReason::Unknown).internal())
}
