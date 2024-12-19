use crate::transport::{connect, CachedConnectionsQueue};
use crate::workers::{Addresses, TcpRecvProcessor, TcpSendWorker};
use crate::{TcpConnectionMode, TcpConnectionOptions, TcpTransport};
use core::fmt;
use core::fmt::Formatter;
use core::str::FromStr;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::flow_control::FlowControlId;
use ockam_core::{Address, Result};
use ockam_node::Context;
use ockam_transport_core::HostnamePort;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as SyncMutex, Weak};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::Instant;
use tracing::debug;

/// Result of [`TcpTransport::connect`] call.
#[derive(Clone, Debug)]
pub struct TcpConnection {
    sender_address: Address,
    receiver_address: Address,
    socket_address: SocketAddr,
    mode: TcpConnectionMode,
    flow_control_id: FlowControlId,
}

impl fmt::Display for TcpConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Socket: {}, Worker: {}, Processor: {}, FlowId: {}",
            self.socket_address, self.sender_address, self.receiver_address, self.flow_control_id
        )
    }
}

impl From<TcpConnection> for Address {
    fn from(value: TcpConnection) -> Self {
        value.sender_address
    }
}

impl TcpConnection {
    /// Constructor
    pub fn new(
        sender_address: Address,
        receiver_address: Address,
        socket_address: SocketAddr,
        mode: TcpConnectionMode,
        flow_control_id: FlowControlId,
    ) -> Self {
        Self {
            sender_address,
            receiver_address,
            socket_address,
            mode,
            flow_control_id,
        }
    }
    /// Stops the [`TcpConnection`], this method must be called to avoid
    /// leakage of the connection.
    /// Simply dropping this object won't close the connection
    pub async fn stop(&self, context: &Context) -> Result<()> {
        context.stop_worker(self.sender_address.clone()).await
    }
    /// Corresponding [`TcpSendWorker`](super::workers::TcpSendWorker) [`Address`] that can be used
    /// in a route to send messages to the other side of the TCP connection
    pub fn sender_address(&self) -> &Address {
        &self.sender_address
    }
    /// Corresponding [`TcpReceiveProcessor`](super::workers::TcpRecvProcessor) [`Address`]
    pub fn receiver_address(&self) -> &Address {
        &self.receiver_address
    }
    /// Corresponding [`SocketAddr`]
    pub fn socket_address(&self) -> &SocketAddr {
        &self.socket_address
    }
    /// Generated fresh random [`FlowControlId`]
    pub fn flow_control_id(&self) -> &FlowControlId {
        &self.flow_control_id
    }
    /// Corresponding [`TcpConnectionMode`]
    pub fn mode(&self) -> TcpConnectionMode {
        self.mode
    }
}

pub(crate) struct ConnectionCollector {
    hostname_port: HostnamePort,
    connections_queue: Weak<CachedConnectionsQueue>,
    read_half: SyncMutex<Option<(Instant, OwnedReadHalf)>>,
    write_half: SyncMutex<Option<OwnedWriteHalf>>,
}

impl ConnectionCollector {
    fn new(hostname_port: HostnamePort, connections_queue: &Arc<CachedConnectionsQueue>) -> Self {
        Self {
            hostname_port,
            connections_queue: Arc::downgrade(connections_queue),
            read_half: SyncMutex::new(None),
            write_half: SyncMutex::new(None),
        }
    }

    pub(crate) fn collect_read_half(&self, last_known_reply: Instant, read_half: OwnedReadHalf) {
        debug!("Collecting read half for {}", self.hostname_port);
        self.read_half
            .lock()
            .unwrap()
            .replace((last_known_reply, read_half));
        self.check_and_push_connection();
    }

    pub(crate) fn collect_write_half(&self, write_half: OwnedWriteHalf) {
        debug!("Collecting write half for {}", self.hostname_port);
        self.write_half.lock().unwrap().replace(write_half);
        self.check_and_push_connection();
    }

    fn check_and_push_connection(&self) {
        let mut read_half = self.read_half.lock().unwrap();
        let mut write_half = self.write_half.lock().unwrap();

        if read_half.is_some() && write_half.is_some() {
            if let Some(connections_queue) = self.connections_queue.upgrade() {
                let (last_known_reply, read_half) = read_half.take().unwrap();
                let write_half = write_half.take().unwrap();

                let mut guard = connections_queue.lock().unwrap();
                let connections = guard.entry(self.hostname_port.clone()).or_default();
                connections.push_back((last_known_reply, read_half, write_half));
            }
        }
    }
}

impl TcpTransport {
    /// Establish an outgoing TCP connection.
    ///
    /// ```rust
    /// use ockam_transport_tcp::{TcpConnectionOptions, TcpListenerOptions, TcpTransport};
    /// # use ockam_node::Context;
    /// # use ockam_core::Result;
    /// # async fn test(ctx: Context) -> Result<()> {
    /// let tcp = TcpTransport::create(&ctx).await?;
    /// tcp.listen("127.0.0.1:8000", TcpListenerOptions::new()).await?; // Listen on port 8000
    /// let connection = tcp.connect("127.0.0.1:5000", TcpConnectionOptions::new()).await?; // and connect to port 5000
    /// # Ok(()) }
    /// ```
    pub async fn connect(
        &self,
        peer: impl Into<String>,
        options: TcpConnectionOptions,
    ) -> Result<TcpConnection> {
        let peer = HostnamePort::from_str(&peer.into())?;

        let (last_known_reply, skip_initialization, read_half, write_half) = {
            let connection = {
                let mut guard = self.connections.lock().unwrap();
                if let Some(connections) = guard.get_mut(&peer) {
                    loop {
                        if let Some((last_known_reply, read_half, write_half)) =
                            connections.pop_front()
                        {
                            let elapsed = last_known_reply.elapsed();
                            if elapsed.as_secs() < 2 {
                                debug!(
                                    "Reusing existing connection to {}, {}ms old",
                                    peer.clone(),
                                    elapsed.as_millis()
                                );
                                break Some((last_known_reply, true, read_half, write_half));
                            }
                        } else {
                            break None;
                        }
                    }
                } else {
                    None
                }
            };

            if let Some(read_write_half) = connection {
                read_write_half
            } else {
                let (read_half, write_half) = connect(&peer).await?;
                (Instant::now(), false, read_half, write_half)
            }
        };

        let socket = read_half
            .peer_addr()
            .map_err(|e| ockam_core::Error::new(Origin::Transport, Kind::Internal, e))?;

        let mode = TcpConnectionMode::Outgoing;
        let addresses = Addresses::generate(mode);

        options.setup_flow_control(self.ctx.flow_controls(), &addresses);
        let flow_control_id = options.flow_control_id.clone();
        let receiver_outgoing_access_control =
            options.create_receiver_outgoing_access_control(self.ctx.flow_controls());

        let connection_collector = {
            if peer.is_localhost() {
                None
            } else {
                Some(Arc::new(ConnectionCollector::new(peer, &self.connections)))
            }
        };

        TcpSendWorker::start(
            &self.ctx,
            self.registry.clone(),
            write_half,
            skip_initialization,
            &addresses,
            socket,
            mode,
            &flow_control_id,
            connection_collector.clone(),
        )
        .await?;

        TcpRecvProcessor::start(
            &self.ctx,
            self.registry.clone(),
            read_half,
            skip_initialization,
            last_known_reply,
            &addresses,
            socket,
            mode,
            &flow_control_id,
            receiver_outgoing_access_control,
            connection_collector,
        )
        .await?;

        Ok(TcpConnection::new(
            addresses.sender_address().clone(),
            addresses.receiver_address().clone(),
            socket,
            mode,
            flow_control_id,
        ))
    }

    /// Interrupt an active TCP connection given its Sender `Address`
    pub async fn disconnect(&self, address: impl Into<Address>) -> Result<()> {
        self.ctx.stop_worker(address.into()).await
    }
}
