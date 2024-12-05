use crate::colors::{color_primary, color_warn};
use crate::kafka::{ConsumerPublishing, ConsumerResolution};
use crate::output::Output;
use minicbor::{CborLen, Decode, Encode};
use ockam_abac::PolicyExpression;
use ockam_core::Address;
use ockam_multiaddr::MultiAddr;
use ockam_transport_core::HostnamePort;
use serde::Serialize;
use std::fmt::Display;
use std::time::Duration;

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartServiceRequest<T> {
    #[n(1)] pub address: String,
    #[n(2)] pub request: T,
}

impl<T> StartServiceRequest<T> {
    pub fn new<S: Into<String>>(request: T, address: S) -> Self {
        Self {
            address: address.into(),
            request,
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn request(&self) -> &T {
        &self.request
    }
}

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct DeleteServiceRequest {
    #[n(1)] addr: String,
}

impl DeleteServiceRequest {
    pub fn new<S: Into<String>>(addr: S) -> Self {
        Self { addr: addr.into() }
    }

    pub fn address(&self) -> Address {
        Address::from(self.addr.clone())
    }
}

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartKafkaCustodianRequest {
    #[n(0)] pub vault: Option<String>,
    #[n(1)] pub producer_policy_expression: Option<PolicyExpression>,
    #[n(2)] pub key_rotation: Duration,
    #[n(3)] pub key_validity: Duration,
    #[n(4)] pub rekey_period: Duration,
}

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartKafkaOutletRequest {
    #[n(1)] bootstrap_server_addr: HostnamePort,
    #[n(2)] tls: bool,
    #[n(3)] policy_expression: Option<PolicyExpression>,
}

impl StartKafkaOutletRequest {
    pub fn new(
        bootstrap_server_addr: HostnamePort,
        tls: bool,
        policy_expression: Option<PolicyExpression>,
    ) -> Self {
        Self {
            bootstrap_server_addr,
            tls,
            policy_expression,
        }
    }

    pub fn bootstrap_server_addr(&self) -> HostnamePort {
        self.bootstrap_server_addr.clone()
    }

    pub fn tls(&self) -> bool {
        self.tls
    }

    pub fn policy_expression(&self) -> Option<PolicyExpression> {
        self.policy_expression.clone()
    }
}

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartKafkaInletRequest {
    #[n(1)] pub bind_address: HostnamePort,
    #[n(2)] pub brokers_port_range: (u16, u16),
    #[n(3)] pub outlet_node_multiaddr: MultiAddr,
    #[n(4)] pub encrypt_content: bool,
    #[n(5)] pub consumer_resolution: ConsumerResolution,
    #[n(6)] pub consumer_publishing: ConsumerPublishing,
    #[n(7)] pub inlet_policy_expression: Option<PolicyExpression>,
    #[n(8)] pub consumer_policy_expression: Option<PolicyExpression>,
    #[n(9)] pub producer_policy_expression: Option<PolicyExpression>,
    #[n(10)] pub encrypted_fields: Vec<String>,
    #[n(11)] pub vault: Option<String>,
}

impl StartKafkaInletRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bind_address: HostnamePort,
        brokers_port_range: impl Into<(u16, u16)>,
        kafka_outlet_route: MultiAddr,
        encrypt_content: bool,
        encrypted_fields: Vec<String>,
        consumer_resolution: ConsumerResolution,
        consumer_publishing: ConsumerPublishing,
        inlet_policy_expression: Option<PolicyExpression>,
        consumer_policy_expression: Option<PolicyExpression>,
        producer_policy_expression: Option<PolicyExpression>,
        vault: Option<String>,
    ) -> Self {
        Self {
            bind_address,
            brokers_port_range: brokers_port_range.into(),
            outlet_node_multiaddr: kafka_outlet_route,
            encrypt_content,
            consumer_resolution,
            consumer_publishing,
            inlet_policy_expression,
            consumer_policy_expression,
            producer_policy_expression,
            encrypted_fields,
            vault,
        }
    }
}

/// Request body when instructing a node to start an Uppercase service
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartUppercaseServiceRequest {
    #[n(1)] pub addr: String,
}

impl StartUppercaseServiceRequest {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }
}

/// Request body when instructing a node to start an Echoer service
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartEchoerServiceRequest {
    #[n(1)] pub addr: String,
}

impl StartEchoerServiceRequest {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }
}

/// Request body when instructing a node to start a Hop service
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartHopServiceRequest {
    #[n(1)] pub addr: String,
}

impl StartHopServiceRequest {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }
}

/// Request body of remote proxy vault service
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct StartRemoteProxyVaultServiceRequest {
    #[n(1)] pub addr: String,
    #[n(2)] pub vault_name: String,
}

impl StartRemoteProxyVaultServiceRequest {
    pub fn new(addr: impl Into<String>, vault_name: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            vault_name: vault_name.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Encode, Decode, CborLen)]
#[rustfmt::skip]
#[cbor(map)]
pub struct ServiceStatus {
    #[n(2)] pub addr: String,
    #[serde(rename = "type")]
    #[n(3)] pub service_type: String,
}

impl ServiceStatus {
    pub fn new(addr: impl Into<String>, service_type: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            service_type: service_type.into(),
        }
    }
}

impl Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} service at {}",
            color_warn(&self.service_type),
            color_primary(&self.addr)
        )
    }
}

impl Output for ServiceStatus {
    fn item(&self) -> crate::Result<String> {
        Ok(self.padded_display())
    }
}
