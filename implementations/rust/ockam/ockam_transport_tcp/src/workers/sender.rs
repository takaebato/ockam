use crate::workers::Addresses;
use crate::{TcpConnectionMode, TcpProtocolVersion, TcpRegistry, TcpSenderInfo, MAX_MESSAGE_SIZE};
use ockam_core::flow_control::FlowControlId;
use ockam_core::{
    async_trait,
    compat::{net::SocketAddr, sync::Arc},
    AllowAll, AllowSourceAddress, DenyAll, LocalMessage,
};
use ockam_core::{Any, Decodable, Mailbox, Mailboxes, Message, Result, Routed, Worker};
use ockam_node::{Context, WorkerBuilder};

use crate::transport::ConnectionCollector;
use crate::transport_message::TcpTransportMessage;
use ockam_transport_core::TransportError;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tracing::{debug, info, instrument, trace, warn};

#[derive(Serialize, Deserialize, Message, Clone)]
pub(crate) enum TcpSendWorkerMsg {
    ConnectionClosed,
}

/// A TCP sending message worker
///
/// Create this worker type by calling
/// [`TcpSendWorker::start_pair`](crate::TcpSendWorker::start_pair)
///
/// This half of the worker is created when spawning a new connection
/// worker pair, and listens for messages from the node message system
/// to dispatch to a remote peer.
pub(crate) struct TcpSendWorker {
    buffer: Vec<u8>,
    registry: TcpRegistry,
    write_half: Option<OwnedWriteHalf>,
    socket_address: SocketAddr,
    addresses: Addresses,
    mode: TcpConnectionMode,
    receiver_flow_control_id: FlowControlId,
    rx_should_be_stopped: bool,
    connection_collector: Option<Arc<ConnectionCollector>>,
    initialized: bool,
}

impl TcpSendWorker {
    /// Create a new `TcpSendWorker`
    #[allow(clippy::too_many_arguments)]
    fn new(
        registry: TcpRegistry,
        write_half: OwnedWriteHalf,
        initialized: bool,
        socket_address: SocketAddr,
        addresses: Addresses,
        mode: TcpConnectionMode,
        receiver_flow_control_id: FlowControlId,
        connection_collector: Option<Arc<ConnectionCollector>>,
    ) -> Self {
        Self {
            buffer: vec![],
            registry,
            write_half: Some(write_half),
            socket_address,
            addresses,
            receiver_flow_control_id,
            mode,
            connection_collector,
            rx_should_be_stopped: true,
            initialized,
        }
    }
}

impl TcpSendWorker {
    /// Create a `(TcpSendWorker, TcpRecvProcessor)` pair that opens and
    /// manages the connection with the given peer
    #[allow(clippy::too_many_arguments)]
    #[instrument(skip_all, name = "TcpSendWorker::start")]
    pub(crate) async fn start(
        ctx: &Context,
        registry: TcpRegistry,
        write_half: OwnedWriteHalf,
        skip_initialization: bool,
        addresses: &Addresses,
        socket_address: SocketAddr,
        mode: TcpConnectionMode,
        receiver_flow_control_id: &FlowControlId,
        connection_collector: Option<Arc<ConnectionCollector>>,
    ) -> Result<()> {
        trace!("Creating new TCP worker pair");
        let sender_worker = Self::new(
            registry,
            write_half,
            skip_initialization,
            socket_address,
            addresses.clone(),
            mode,
            receiver_flow_control_id.clone(),
            connection_collector,
        );

        let main_mailbox = Mailbox::new(
            addresses.sender_address().clone(),
            Arc::new(AllowAll),
            Arc::new(DenyAll),
        );

        let internal_mailbox = Mailbox::new(
            addresses.sender_internal_address().clone(),
            Arc::new(AllowSourceAddress(
                addresses.receiver_internal_address().clone(),
            )),
            Arc::new(DenyAll),
        );

        WorkerBuilder::new(sender_worker)
            .with_mailboxes(Mailboxes::new(main_mailbox.clone(), vec![internal_mailbox]))
            .terminal(addresses.sender_address().clone())
            .start(ctx)
            .await?;

        Ok(())
    }

    #[instrument(skip_all, name = "TcpSendWorker::stop")]
    async fn stop(&self, ctx: &Context) -> Result<()> {
        ctx.stop_worker(self.addresses.sender_address().clone())
            .await?;

        Ok(())
    }

    fn serialize_message(&mut self, local_message: LocalMessage) -> Result<()> {
        // Create a message buffer with prepended length
        let transport_message = TcpTransportMessage::from(local_message);

        let expected_payload_len = minicbor::len(&transport_message);

        const LENGTH_VALUE_SIZE: usize = 4; // u32

        // This buffer starts from 0 length, and grows when we receive a bigger message.
        self.buffer.clear();
        self.buffer
            .reserve(LENGTH_VALUE_SIZE + expected_payload_len);

        // Let's write zeros instead of actual length, since we don't know the exact size yet.
        self.buffer.extend_from_slice(&[0u8; LENGTH_VALUE_SIZE]);

        // Append encoded payload
        minicbor::encode(&transport_message, &mut self.buffer)
            .map_err(|_| TransportError::Encoding)?;

        // Should not ever happen...
        if self.buffer.len() < LENGTH_VALUE_SIZE {
            return Err(TransportError::Encoding)?;
        }

        let payload_len = self.buffer.len() - LENGTH_VALUE_SIZE;

        if payload_len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageLengthExceeded)?;
        }

        let payload_len_u32 =
            u32::try_from(payload_len).map_err(|_| TransportError::MessageLengthExceeded)?;

        // Replace zeros with actual length
        self.buffer[..LENGTH_VALUE_SIZE].copy_from_slice(&payload_len_u32.to_be_bytes());
        trace!("Sending {payload_len_u32} bytes");

        Ok(())
    }
}

#[async_trait]
impl Worker for TcpSendWorker {
    type Context = Context;
    type Message = Any;

    #[instrument(skip_all, name = "TcpSendWorker::initialize")]
    async fn initialize(&mut self, ctx: &mut Self::Context) -> Result<()> {
        ctx.set_cluster(crate::CLUSTER_NAME).await?;

        self.registry.add_sender_worker(TcpSenderInfo::new(
            self.addresses.sender_address().clone(),
            self.addresses.receiver_address().clone(),
            self.socket_address,
            self.mode,
            self.receiver_flow_control_id.clone(),
        ));

        // First thing sends our protocol version
        if self.initialized {
            return Ok(());
        }

        if let Some(write_half) = self.write_half.as_mut() {
            if write_half
                .write_u8(TcpProtocolVersion::V1.into())
                .await
                .is_err()
            {
                warn!(
                    "Failed to send protocol version to peer {}",
                    self.socket_address
                );
                self.write_half.take();
                self.stop(ctx).await?;

                return Ok(());
            }
        } else {
            self.stop(ctx).await?;
            return Err(TransportError::ConnectionDrop)?;
        }

        self.initialized = true;

        Ok(())
    }

    #[instrument(skip_all, name = "TcpSendWorker::shutdown")]
    async fn shutdown(&mut self, ctx: &mut Self::Context) -> Result<()> {
        if self.initialized {
            if let Some(connection_collector) = self.connection_collector.as_ref() {
                if let Some(write_half) = self.write_half.take() {
                    connection_collector.collect_write_half(write_half);
                } else {
                    debug!("Connection closed, no read write to collect");
                }
            }
        }

        self.registry
            .remove_sender_worker(self.addresses.sender_address());

        if self.rx_should_be_stopped {
            let _ = ctx
                .stop_processor(self.addresses.receiver_address().clone())
                .await;
        }

        Ok(())
    }

    // TcpSendWorker will receive messages from the TcpRouter to send
    // across the TcpStream to our friend
    #[instrument(skip_all, name = "TcpSendWorker::handle_message", fields(worker = %ctx.address()))]
    async fn handle_message(
        &mut self,
        ctx: &mut Context,
        msg: Routed<Self::Message>,
    ) -> Result<()> {
        let recipient = msg.msg_addr();
        if &recipient == self.addresses.sender_internal_address() {
            let msg = TcpSendWorkerMsg::decode(msg.payload())?;

            match msg {
                TcpSendWorkerMsg::ConnectionClosed => {
                    info!(
                        "Stopping sender due to closed connection {}",
                        self.socket_address
                    );
                    // No need to stop Receiver as it notified us about connection drop and will
                    // stop itself
                    self.rx_should_be_stopped = false;
                    self.write_half.take();
                    self.stop(ctx).await?;

                    return Ok(());
                }
            }
        } else {
            let mut local_message = msg.into_local_message();
            // Remove our own address from the route so the other end
            // knows what to do with the incoming message
            local_message = local_message.pop_front_onward_route()?;

            if let Err(err) = self.serialize_message(local_message) {
                // Close the stream
                self.write_half.take();
                self.stop(ctx).await?;

                return Err(err);
            };

            if let Some(write_half) = self.write_half.as_mut() {
                if write_half.write_all(&self.buffer).await.is_err() {
                    warn!("Failed to send message to peer {}", self.socket_address);
                    self.write_half.take();
                    self.stop(ctx).await?;

                    return Ok(());
                }
            } else {
                self.stop(ctx).await?;
                return Err(TransportError::ConnectionDrop)?;
            }
        }

        Ok(())
    }
}
