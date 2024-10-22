#![recursion_limit = "256"]

use ockam_api::nodes::service::SecureChannelType;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio::time::timeout;

use ockam_api::nodes::models::portal::OutletAccessControl;
use ockam_api::test_utils::{start_tcp_echo_server, TestNode};
use ockam_core::env::FromString;
use ockam_core::{route, Address, AllowAll, NeutralMessage};
use ockam_multiaddr::MultiAddr;
use ockam_transport_core::HostnamePort;

/// These tests serve as a benchmark for the message roundtrip latency.
/// In order for the result to be reliable, use the --profile release
/// flag when running the tests.
/// `cargo test --test latency --release -- --ignored --show-output`
#[ignore]
#[test]
pub fn measure_message_latency_two_nodes() -> ockam_core::Result<()> {
    let runtime = Arc::new(Runtime::new().unwrap());
    let runtime_cloned = runtime.clone();
    std::env::remove_var("OCKAM_LOG_LEVEL");

    runtime_cloned.block_on(async move {
        let test_body = async move {
            TestNode::clean().await?;
            let mut first_node = TestNode::create(runtime.clone(), None).await;
            let second_node = TestNode::create(runtime.clone(), None).await;

            let secure_channel = first_node
                .node_manager
                .create_secure_channel(
                    &first_node.context,
                    second_node
                        .listen_address()
                        .await
                        .multi_addr()
                        .unwrap()
                        .concat(&MultiAddr::from_string("/service/api").unwrap())
                        .unwrap(),
                    None,
                    None,
                    None,
                    None,
                    SecureChannelType::KeyExchangeAndMessages,
                )
                .await
                .unwrap();

            let ping_route = route![secure_channel.encryptor_address().address(), "echo"];
            let next = ping_route.next().unwrap();

            if let Some(flow_control_id) = first_node
                .context
                .flow_controls()
                .find_flow_control_with_producer_address(next)
                .map(|x| x.flow_control_id().clone())
            {
                first_node
                    .context
                    .flow_controls()
                    .add_consumer(first_node.context.address(), &flow_control_id);
            }

            let payload = NeutralMessage::from(vec![1, 2, 3, 4]);

            // warm up buffers, cache, etc...
            for _ in 0..100 {
                first_node
                    .context
                    .send(ping_route.clone(), payload.clone())
                    .await
                    .unwrap();
                first_node
                    .context
                    .receive::<NeutralMessage>()
                    .await
                    .unwrap();
            }

            let now = Instant::now();
            for _ in 0..10_000 {
                first_node
                    .context
                    .send(ping_route.clone(), payload.clone())
                    .await
                    .unwrap();
                first_node
                    .context
                    .receive::<NeutralMessage>()
                    .await
                    .unwrap();
            }
            let elapsed = now.elapsed();
            println!(
                "single message, roundtrip latency: {:?}",
                elapsed.div_f32(10_000f32)
            );

            first_node.context.stop().await?;
            second_node.context.stop().await?;

            Ok(())
        };

        timeout(Duration::from_secs(30), test_body).await.unwrap()
    })
}

#[ignore]
#[test]
pub fn measure_buffer_latency_two_nodes_portal() -> ockam_core::Result<()> {
    let runtime = Arc::new(Runtime::new().unwrap());
    let runtime_cloned = runtime.clone();
    std::env::remove_var("OCKAM_LOG_LEVEL");

    runtime_cloned.block_on(async move {
        let test_body = async move {
            let echo_server_handle = start_tcp_echo_server().await;

            TestNode::clean().await?;
            let first_node = TestNode::create(runtime.clone(), None).await;
            let second_node = TestNode::create(runtime.clone(), None).await;

            let _outlet_status = second_node
                .node_manager
                .create_outlet(
                    &second_node.context,
                    echo_server_handle.chosen_addr.clone(),
                    false,
                    Some(Address::from_string("outlet")),
                    true,
                    OutletAccessControl::AccessControl((Arc::new(AllowAll), Arc::new(AllowAll))),
                    false,
                )
                .await?;

            let second_node_listen_address = second_node.listen_address().await;

            // create inlet in the first node pointing to the second one
            let inlet_status = first_node
                .node_manager
                .create_inlet(
                    &first_node.context,
                    HostnamePort::new("127.0.0.1", 0),
                    route![],
                    route![],
                    second_node_listen_address
                        .multi_addr()?
                        .concat(&MultiAddr::from_string("/secure/api/service/outlet")?)?,
                    "inlet_alias".to_string(),
                    None,
                    None,
                    None,
                    true,
                    None,
                    false,
                    false,
                    false,
                    None,
                )
                .await?;

            // connect to inlet_status.bind_addr and send dummy payload
            let mut socket = TcpStream::connect(inlet_status.bind_addr.clone())
                .await
                .unwrap();

            socket.set_nodelay(true).unwrap();

            let mut buffer = [0u8; 5];

            for _ in 0..100 {
                socket.write_all(b"hello").await.unwrap();
                socket.read_exact(&mut buffer).await.unwrap();
            }

            let now = Instant::now();
            for _ in 0..10_000 {
                socket.write_all(b"hello").await.unwrap();
                socket.read_exact(&mut buffer).await.unwrap();
            }
            let elapsed = now.elapsed();
            println!(
                "short payload, roundtrip latency: {:?}",
                elapsed.div_f32(10_000f32)
            );

            first_node.context.stop().await?;
            second_node.context.stop().await?;

            Ok(())
        };

        timeout(Duration::from_secs(30), test_body).await.unwrap()
    })
}
