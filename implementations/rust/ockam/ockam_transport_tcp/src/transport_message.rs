use minicbor::{CborLen, Decode, Encode};
use ockam_core::compat::string::String;
use ockam_core::{CowBytes, LocalMessage, Route};

/// TCP transport message type.
#[derive(Debug, Clone, Eq, PartialEq, Encode, Decode, CborLen)]
#[rustfmt::skip]
pub struct TcpTransportMessage<'a> {
    #[n(0)] pub onward_route: Route,
    #[n(1)] pub return_route: Route,
    #[b(2)] pub payload: CowBytes<'a>,
    #[n(3)] pub tracing_context: Option<String>,
}

impl<'a> TcpTransportMessage<'a> {
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
}

impl From<TcpTransportMessage<'_>> for LocalMessage {
    fn from(value: TcpTransportMessage) -> Self {
        LocalMessage::new()
            .with_onward_route(value.onward_route)
            .with_return_route(value.return_route)
            .with_payload(value.payload.into_owned())
    }
}

impl From<LocalMessage> for TcpTransportMessage<'_> {
    fn from(value: LocalMessage) -> Self {
        Self::new(
            value.onward_route,
            value.return_route,
            CowBytes::from(value.payload),
            None,
        )
    }
}
