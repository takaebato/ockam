use minicbor::{CborLen, Decode, Encode};
use ockam_core::compat::string::String;
use ockam_core::{CowBytes, LocalMessage, Route};

/// Ockam Routing Message that we want to send to the other side via UDP.
/// This message could be larger than a supported UDP datagram, therefore can be split into
/// multiple [`UdpTransportMessage`]s
#[derive(Debug, Clone, Eq, PartialEq, Encode, Decode, CborLen)]
#[rustfmt::skip]
pub struct UdpRoutingMessage<'a> {
    #[n(0)] pub onward_route: Route,
    #[n(1)] pub return_route: Route,
    #[b(2)] pub payload: CowBytes<'a>,
    #[n(3)] pub tracing_context: Option<String>,
}

impl<'a> UdpRoutingMessage<'a> {
    /// Constructor.
    pub fn new(
        onward_route: Route,
        return_route: Route,
        payload: CowBytes<'a>,
        tracing_context: Option<String>,
    ) -> Self {
        Self {
            onward_route,
            return_route,
            payload,
            tracing_context,
        }
    }

    pub fn into_owned(self) -> UdpRoutingMessage<'static> {
        UdpRoutingMessage {
            onward_route: self.onward_route,
            return_route: self.return_route,
            payload: self.payload.into_owned().into(),
            tracing_context: self.tracing_context,
        }
    }
}

impl From<UdpRoutingMessage<'_>> for LocalMessage {
    fn from(value: UdpRoutingMessage) -> Self {
        let local_message = LocalMessage::new();

        local_message
            .with_onward_route(value.onward_route)
            .with_return_route(value.return_route)
            .with_payload(value.payload.into_owned())
    }
}

impl From<LocalMessage> for UdpRoutingMessage<'_> {
    fn from(value: LocalMessage) -> Self {
        let routing_message = Self::new(
            value.onward_route,
            value.return_route,
            CowBytes::from(value.payload),
            None,
        );
        routing_message
    }
}
