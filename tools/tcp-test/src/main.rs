use clap::{Args, Parser, Subcommand};
use std::collections::VecDeque;
use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Notify;
use tokio_rustls::rustls;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use tokio_rustls::rustls::{DigitallySignedStruct, Error, SignatureScheme};

#[derive(Debug, Args, Clone)]
struct EchoCommand {
    bind_port: u16,
    #[arg(long, default_value = "false")]
    tls: bool,
}

#[derive(Debug, Args, Clone)]
struct NullCommand {
    bind_port: u16,
    #[arg(long, default_value = "false")]
    tls: bool,
}

#[derive(Debug, Args, Clone)]
struct LatencyCommand {
    address: SocketAddr,
    #[arg(long, default_value = "10")]
    count: u32,
    #[arg(long, default_value = "false")]
    tls: bool,
}

#[derive(Debug, Args, Clone)]
struct FloodCommand {
    address: SocketAddr,
    #[arg(long, default_value = "0")]
    max: u32,
}

#[derive(Debug, Args, Clone)]
struct ThroughputCommand {
    address: SocketAddr,
    #[arg(long, default_value = "false")]
    tls: bool,
}

#[derive(Subcommand, Debug, Clone)]
enum Action {
    /// Run a TCP echo server
    Echo(EchoCommand),
    /// Run a TCP server, discards all incoming data
    Null(NullCommand),
    /// Measure the latency of a TCP connection
    Latency(LatencyCommand),
    /// Flood a TCP port with connections until it fails
    Flood(FloodCommand),
    /// Measure the throughput of TCP connection
    Throughput(ThroughputCommand),
}

#[derive(Debug, Parser, Clone)]
#[command(name = "tcp-tester")]
struct Main {
    /// Action to perform
    #[command(subcommand)]
    action: Action,
}

#[tokio::main]
async fn main() {
    let main: Main = Main::parse();
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    match main.action {
        Action::Echo(cmd) => start_server(cmd.bind_port, cmd.tls, ServerMode::Echo).await,
        Action::Null(cmd) => start_server(cmd.bind_port, cmd.tls, ServerMode::Null).await,
        Action::Latency(cmd) => latency(cmd).await,
        Action::Flood(cmd) => flood(cmd).await,
        Action::Throughput(cmd) => throughput(cmd).await,
    }
}

async fn throughput(command: ThroughputCommand) {
    let connector = if command.tls {
        let config = Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoTlsValidation))
                .with_no_client_auth(),
        );
        Some(tokio_rustls::TlsConnector::from(config))
    } else {
        None
    };

    let stream = tokio::net::TcpStream::connect(command.address)
        .await
        .unwrap();
    stream.set_nodelay(true).unwrap();
    let mut stream_write: Box<dyn AsyncWrite + Unpin + Send>;
    let mut stream_read: Box<dyn AsyncRead + Unpin + Send>;

    if let Some(connector) = &connector {
        let result = connector
            .connect(ServerName::IpAddress(command.address.ip().into()), stream)
            .await;
        match result {
            Ok(stream) => {
                let (r, w) = tokio::io::split(stream);
                stream_read = Box::new(r);
                stream_write = Box::new(w);
            }
            Err(error) => {
                println!("Failed to connect: {:?}", error);
                return;
            }
        }
    } else {
        let (r, w) = tokio::io::split(stream);
        stream_read = Box::new(r);
        stream_write = Box::new(w);
    }

    // read as much as possible and discard the result
    tokio::spawn(async move {
        let mut incoming_buffer = [0u8; 64 * 1024];
        loop {
            let result = stream_read.read(&mut incoming_buffer).await;
            if let Err(err) = result {
                println!("Failed to read: {:?}", err);
                return;
            }
        }
    });

    // read and report the speed
    let outgoing_buffer = [0u8; 64 * 1024];
    let mut total_bytes = 0;
    let mut iterations = 0;

    let mut start = Instant::now();
    loop {
        let result = stream_write.write_all(&outgoing_buffer).await;
        iterations += 1;
        match result {
            Ok(()) => {
                total_bytes += outgoing_buffer.len();
            }
            Err(err) => {
                println!("Failed to write: {:?}", err);
                return;
            }
        }

        if iterations > 10_000 {
            iterations = 0;
            let elapsed = start.elapsed();
            let seconds = elapsed.as_secs();
            if seconds >= 1 {
                println!(
                    "Throughput: {}Gbits/s",
                    (total_bytes as f32 / elapsed.as_secs_f32()) * 8.0 / 1_000_000_000.0
                );
                start = Instant::now();
                total_bytes = 0;
            }
        }
    }
}

#[derive(Clone)]
enum ServerMode {
    Echo,
    Null,
}

async fn start_server(bind_port: u16, tls: bool, mode: ServerMode) {
    let bind_address = SocketAddr::new(IpAddr::from([0, 0, 0, 0]), bind_port);
    let listener = tokio::net::TcpListener::bind(bind_address).await.unwrap();

    let acceptor = if tls {
        println!("Listening TLS on {}", listener.local_addr().unwrap());
        let self_signed = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![
                        CertificateDer::from_pem_slice(self_signed.cert.pem().as_bytes()).unwrap(),
                    ],
                    PrivateKeyDer::try_from(self_signed.key_pair.serialize_der()).unwrap(),
                )
                .unwrap(),
        );
        Some(tokio_rustls::TlsAcceptor::from(config))
    } else {
        println!("Listening on {}", listener.local_addr().unwrap());
        None
    };

    loop {
        if let Ok((stream, _)) = listener.accept().await {
            stream.set_nodelay(true).unwrap();
            if let Some(acceptor) = &acceptor {
                let result = acceptor.accept(stream).await;
                match result {
                    Ok(stream) => {
                        let (r, w) = tokio::io::split(stream);
                        let mode = mode.clone();
                        tokio::spawn(async move { server(r, w, mode).await });
                    }
                    Err(err) => {
                        println!("Failed to accept TLS connection: {:?}", err);
                    }
                }
            } else {
                let (r, w) = tokio::io::split(stream);
                let mode = mode.clone();
                tokio::spawn(async move { server(r, w, mode).await });
            }
        } else {
            println!("Failed to accept connection");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}

async fn server(
    stream_reader: impl AsyncRead + Unpin + Send + 'static,
    stream_writer: impl AsyncWrite + Unpin + Send + 'static,
    mode: ServerMode,
) {
    match mode {
        ServerMode::Echo => {
            fast_echo(stream_reader, stream_writer).await;
        }
        ServerMode::Null => {
            let mut stream_reader = stream_reader;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let result = stream_reader.read(&mut buffer).await;
                if let Err(err) = result {
                    println!("Failed to read: {:?}", err);
                    return;
                }
            }
        }
    }
}

async fn fast_echo(
    mut stream_reader: impl AsyncRead + Unpin + Send + 'static,
    mut stream_writer: impl AsyncWrite + Unpin + Send + 'static,
) {
    let new_outgoing_buffer_event = Arc::new(Notify::new());
    let new_incoming_buffer_event = Arc::new(Notify::new());

    let incoming_buffers = {
        let mut queue = VecDeque::with_capacity(4);
        for _ in 0..queue.capacity() {
            queue.push_back(vec![0u8; 64 * 1024]);
        }
        Arc::new(Mutex::new(queue))
    };

    let outgoing_buffers = Arc::new(Mutex::new(VecDeque::with_capacity(8)));

    {
        let incoming_buffers = incoming_buffers.clone();
        let new_incoming_buffer = new_incoming_buffer_event.clone();
        let outgoing_buffers = outgoing_buffers.clone();
        let new_outgoing_buffer = new_outgoing_buffer_event.clone();

        tokio::spawn(async move {
            loop {
                let incoming_buffer = incoming_buffers.lock().unwrap().pop_front();
                if let Some(mut incoming_buffer) = incoming_buffer {
                    let result = stream_reader.read(&mut incoming_buffer).await;
                    match result {
                        Ok(bytes) => {
                            outgoing_buffers
                                .lock()
                                .unwrap()
                                .push_back((bytes, incoming_buffer));
                            new_outgoing_buffer.notify_one();
                        }
                        Err(err) => {
                            println!("Failed to read: {:?}", err);
                            return;
                        }
                    }
                } else {
                    new_incoming_buffer.notified().await;
                }
            }
        });
    }

    loop {
        let outgoing_buffer = outgoing_buffers.lock().unwrap().pop_front();
        if let Some((bytes, outgoing_buffer)) = outgoing_buffer {
            let result = stream_writer.write_all(&outgoing_buffer[..bytes]).await;
            match result {
                Ok(()) => {
                    incoming_buffers.lock().unwrap().push_back(outgoing_buffer);
                    new_incoming_buffer_event.notify_one();
                }
                Err(err) => {
                    println!("Failed to write: {:?}", err);
                    return;
                }
            }
        } else {
            new_outgoing_buffer_event.notified().await;
        }
    }
}

struct NoTlsValidation;

impl Debug for NoTlsValidation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoTlsValidation").finish()
    }
}

impl ServerCertVerifier for NoTlsValidation {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

async fn latency(command: LatencyCommand) {
    let mut first_rtt_measurements = Vec::new();
    let mut second_rtt_measurements = Vec::new();
    let mut incoming_buffer = [0u8; 5];
    let tls_connector = if command.tls {
        let config = Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoTlsValidation))
                .with_no_client_auth(),
        );
        Some(tokio_rustls::TlsConnector::from(config))
    } else {
        None
    };

    for _ in 0..command.count {
        let result = tokio::net::TcpStream::connect(command.address).await;
        match result {
            Ok(stream) => {
                stream.set_nodelay(true).unwrap();

                let mut stream_write: Box<dyn AsyncWrite + Unpin>;
                let mut stream_read: Box<dyn AsyncRead + Unpin>;

                let start = Instant::now();

                if let Some(tls_connector) = &tls_connector {
                    let result = tls_connector
                        .connect(ServerName::IpAddress(command.address.ip().into()), stream)
                        .await;
                    match result {
                        Ok(stream) => {
                            let (r, w) = tokio::io::split(stream);
                            stream_read = Box::new(r);
                            stream_write = Box::new(w);
                        }
                        Err(error) => {
                            println!("Failed to connect: {:?}", error);
                            continue;
                        }
                    }
                } else {
                    let (r, w) = tokio::io::split(stream);
                    stream_read = Box::new(r);
                    stream_write = Box::new(w);
                }

                let result = stream_write.write_all("hello".as_bytes()).await;
                if let Err(err) = result {
                    println!("Failed to write: {:?}", err);
                    continue;
                }
                let result = stream_read.read_exact(&mut incoming_buffer).await;
                if let Err(err) = result {
                    println!("Failed to read: {:?}", err);
                    continue;
                }
                let first_rtt = start.elapsed();
                first_rtt_measurements.push(first_rtt);

                let result = stream_write.write_all("hello".as_bytes()).await;
                if let Err(err) = result {
                    println!("Failed to write: {:?}", err);
                    continue;
                }
                let result = stream_read.read_exact(&mut incoming_buffer).await;
                if let Err(err) = result {
                    println!("Failed to read: {:?}", err);
                    continue;
                }
                let second_rtt = start.elapsed() - first_rtt;
                second_rtt_measurements.push(second_rtt);
                println!(
                    "Connection + First RTT: {}ms {}us, Second RTT: {}ms {}us",
                    first_rtt.as_millis(),
                    first_rtt.as_micros(),
                    second_rtt.as_millis(),
                    second_rtt.as_micros()
                );
            }
            Err(err) => {
                println!("Failed to connect: {:?}", err);
            }
        }
    }

    let total: std::time::Duration = first_rtt_measurements.iter().sum();
    let first_average = total / command.count;
    let total: std::time::Duration = second_rtt_measurements.iter().sum();
    let second_average = total / command.count;
    println!(
        "Average - Connection + First RTT: {}ms {}us, Second RTT: {}ms {}us",
        first_average.as_millis(),
        first_average.as_micros(),
        second_average.as_millis(),
        second_average.as_micros(),
    );
}

async fn flood(command: FloodCommand) {
    let mut connections = Vec::new();
    let mut counter = 0;
    loop {
        let result = tokio::net::TcpStream::connect(command.address).await;
        match result {
            Ok(stream) => {
                stream.set_nodelay(true).unwrap();
                connections.push(stream);
                counter += 1;
                if command.max > 0 && counter >= command.max {
                    break;
                }
            }
            Err(err) => {
                println!("Failed to connect: {:?}", err);
                break;
            }
        }
    }

    println!("Flooded {} connections", connections.len());
    println!("Pinging each...");

    let mut failed = 0;
    let mut incoming_buffer = [0u8; 5];
    for mut stream in connections {
        let result = stream.write_all("hello".as_bytes()).await;
        if let Err(err) = result {
            println!("Failed to write: {:?}", err);
            failed += 1;
            continue;
        }
        let result = stream.read_exact(&mut incoming_buffer).await;
        if let Err(err) = result {
            println!("Failed to read: {:?}", err);
            failed += 1;
        }
    }

    if failed == 0 {
        println!("All connections succeeded");
    } else {
        println!("{} connections failed", failed);
    }
}
