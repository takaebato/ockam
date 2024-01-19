use sqlx::__rt::timeout;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tracing::{error, info};

pub struct EchoServerHandle {
    pub chosen_addr: SocketAddr,
    close: Arc<AtomicBool>,
}

impl Drop for EchoServerHandle {
    fn drop(&mut self) {
        self.close.store(true, Ordering::Relaxed);
    }
}

#[must_use = "listener closed when dropped"]
pub async fn start_tcp_echo_server() -> EchoServerHandle {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("Failed to bind server to address");

    let chosen_addr = listener.local_addr().unwrap();
    let close = Arc::new(AtomicBool::new(false));

    {
        let close = close.clone();
        tokio::spawn(async move {
            loop {
                let result = match timeout(Duration::from_millis(200), listener.accept()).await {
                    Ok(result) => result,
                    Err(_) => {
                        if close.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    }
                };

                let (mut socket, _) = result.expect("Failed to accept connection");
                tokio::spawn(async move {
                    let mut buf = vec![0; 1024];
                    loop {
                        let n = match socket.read(&mut buf).await {
                            // socket closed
                            Ok(n) if n == 0 => return,
                            Ok(n) => n,
                            Err(e) => {
                                println!("Failed to read from socket; err = {:?}", e);
                                return;
                            }
                        };

                        // Write the data back
                        if let Err(e) = socket.write_all(&buf[0..n]).await {
                            println!("Failed to write to socket; err = {:?}", e);
                            return;
                        }
                    }
                });
            }
        });
    }

    EchoServerHandle { chosen_addr, close }
}

pub struct PassthroughServerHandle {
    pub chosen_addr: SocketAddr,
    pub destination: SocketAddr,
    close: Arc<AtomicBool>,
}

impl Drop for PassthroughServerHandle {
    fn drop(&mut self) {
        self.close.store(true, Ordering::Relaxed);
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum Disruption {
    None,
    LimitBandwidth(usize),
    DropPacketsAfter(usize),
    PacketsOutOfOrderAfter(usize),
}

#[must_use = "listener closed when dropped"]
pub async fn start_passthrough_server(
    destination: &str,
    disruption: Disruption,
) -> PassthroughServerHandle {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let socket = TcpSocket::new_v4().unwrap();

    // to reduce the impact of this passthrough server on the benchmarks and tests
    // we set the receive buffer size to 1KB
    socket.set_recv_buffer_size(1024).unwrap();
    socket.bind(addr).expect("Failed to bind server to address");

    let listener = socket.listen(32).unwrap();

    let destination = destination
        .parse()
        .expect("Failed to parse destination address");

    let chosen_addr = listener.local_addr().unwrap();
    let close = Arc::new(AtomicBool::new(false));

    {
        let close = close.clone();
        let disruption = *&disruption;
        tokio::spawn(async move {
            loop {
                let result = match timeout(Duration::from_millis(200), listener.accept()).await {
                    Ok(result) => result,
                    Err(_) => {
                        if close.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    }
                };

                let (incoming_socket, _) = result.expect("Failed to accept connection");
                tokio::spawn(async move {
                    let outgoing_socket = match TcpStream::connect(destination).await {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Failed to connect to destination; err = {:?}", e);
                            return;
                        }
                    };
                    let (incoming_read, incoming_write) = incoming_socket.into_split();
                    let (outgoing_read, outgoing_write) = outgoing_socket.into_split();

                    match disruption {
                        Disruption::None => {
                            tokio::spawn(async move {
                                relay_stream_limit_bandwidth(incoming_read, outgoing_write, None)
                                    .await
                            });
                            tokio::spawn(async move {
                                relay_stream_limit_bandwidth(outgoing_read, incoming_write, None)
                                    .await
                            });
                        }
                        Disruption::LimitBandwidth(bytes_per_second) => {
                            tokio::spawn(async move {
                                relay_stream_limit_bandwidth(
                                    incoming_read,
                                    outgoing_write,
                                    Some(bytes_per_second),
                                )
                                .await
                            });
                            tokio::spawn(async move {
                                relay_stream_limit_bandwidth(
                                    outgoing_read,
                                    incoming_write,
                                    Some(bytes_per_second),
                                )
                                .await
                            });
                        }
                        Disruption::DropPacketsAfter(drop_packets_after) => {
                            tokio::spawn(async move {
                                relay_stream_drop_packets(
                                    incoming_read,
                                    outgoing_write,
                                    drop_packets_after,
                                )
                                .await
                            });
                            tokio::spawn(async move {
                                relay_stream_drop_packets(
                                    outgoing_read,
                                    incoming_write,
                                    drop_packets_after,
                                )
                                .await
                            });
                        }
                        Disruption::PacketsOutOfOrderAfter(packet_out_of_order_after) => {
                            tokio::spawn(async move {
                                relay_stream_packets_out_of_order(
                                    incoming_read,
                                    outgoing_write,
                                    packet_out_of_order_after,
                                )
                                .await
                            });
                            tokio::spawn(async move {
                                relay_stream_packets_out_of_order(
                                    outgoing_read,
                                    incoming_write,
                                    packet_out_of_order_after,
                                )
                                .await
                            });
                        }
                    }
                });
            }
        });
    }

    PassthroughServerHandle {
        chosen_addr,
        destination,
        close,
    }
}

async fn relay_stream_limit_bandwidth(
    mut read_half: OwnedReadHalf,
    mut write_half: OwnedWriteHalf,
    max_bytes_per_second: Option<usize>,
) {
    let mut bytes_counter = 0;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = match read_half.read(&mut buffer).await {
            // socket closed
            Ok(n) if n == 0 => return,
            Ok(n) => n,
            Err(e) => {
                error!("Failed to read from socket; err = {:?}", e);
                return;
            }
        };

        if let Err(e) = write_half.write_all(&buffer[0..read]).await {
            error!("Failed to write to socket; err = {:?}", e);
            return;
        }

        bytes_counter += read;
        if let Some(max_bytes_per_second) = max_bytes_per_second {
            if bytes_counter >= max_bytes_per_second {
                bytes_counter = 0;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn relay_stream_drop_packets(
    mut read_half: OwnedReadHalf,
    mut write_half: OwnedWriteHalf,
    drop_packets_after: usize,
) {
    let mut packet_counter: usize = 0;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        // read the first 2 bytes with the packet size
        match read_half.read_exact(&mut buffer[0..2]).await {
            // socket closed
            Ok(n) if n == 0 => return,
            Ok(n) => {
                if n != 2 {
                    error!(
                        "Failed to read from socket; err = {:?}",
                        "incomplete packet size"
                    );
                    return;
                }
            }
            Err(e) => {
                error!("Failed to read from socket; err = {:?}", e);
                return;
            }
        };

        let packet_size = (&buffer[0..2]).read_u16().await.unwrap() + 2;
        match read_half
            .read_exact(&mut buffer[2..packet_size as usize])
            .await
        {
            // socket closed
            Ok(n) if n == 0 => return,
            Ok(_) => {}
            Err(e) => {
                error!("Failed to read from socket; err = {:?}", e);
                return;
            }
        }

        if packet_counter <= drop_packets_after || packet_counter % 2 == 0 {
            if let Err(e) = write_half.write_all(&buffer[0..packet_size as usize]).await {
                error!("Failed to write to socket; err = {:?}", e);
                return;
            }
        } else {
            info!("Dropping packet {packet_counter} of size {packet_size}");
        }

        packet_counter += 1;
    }
}

async fn relay_stream_packets_out_of_order(
    mut read_half: OwnedReadHalf,
    mut write_half: OwnedWriteHalf,
    packet_out_of_order_after: usize,
) {
    let mut packet_counter: usize = 0;
    let mut previus_buffer: Option<Vec<u8>> = None;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        // read the first 2 bytes with the packet size
        match read_half.read_exact(&mut buffer[0..2]).await {
            // socket closed
            Ok(n) if n == 0 => return,
            Ok(n) => {
                if n != 2 {
                    error!(
                        "Failed to read from socket; err = {:?}",
                        "incomplete packet size"
                    );
                    return;
                }
            }
            Err(e) => {
                error!("Failed to read from socket; err = {:?}", e);
                return;
            }
        };

        let packet_size = (&buffer[0..2]).read_u16().await.unwrap() + 2;
        match read_half
            .read_exact(&mut buffer[2..packet_size as usize])
            .await
        {
            // socket closed
            Ok(n) if n == 0 => return,
            Ok(_) => {}
            Err(e) => {
                error!("Failed to read from socket; err = {:?}", e);
                return;
            }
        };

        if packet_counter > packet_out_of_order_after {
            // write the packet and then the previous one
            if packet_counter % 2 == 0 {
                if let Err(e) = write_half.write_all(&buffer[0..packet_size as usize]).await {
                    error!("Failed to write to socket; err = {:?}", e);
                    return;
                }

                if let Some(previous_buffer) = previus_buffer.take() {
                    if let Err(e) = write_half.write_all(&previous_buffer).await {
                        error!("Failed to write to socket; err = {:?}", e);
                        return;
                    }
                }
            } else {
                info!("Reversing order of packet {packet_counter} of size {packet_size}");
                previus_buffer = Some(buffer[0..packet_size as usize].to_vec());
            }
        } else {
            if let Err(e) = write_half.write_all(&buffer[0..packet_size as usize]).await {
                error!("Failed to write to socket; err = {:?}", e);
                return;
            }
        }

        packet_counter += 1;
    }
}
