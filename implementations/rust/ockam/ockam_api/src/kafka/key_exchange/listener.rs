use crate::DefaultAddress;
use minicbor::{CborLen, Decode, Encode};
use ockam::identity::TimestampInSeconds;
use ockam_core::flow_control::FlowControlId;
use ockam_core::{
    async_trait, Address, Decodable, Encodable, Encoded, IncomingAccessControl, Message,
    OutgoingAccessControl, Routed, Worker,
};
use ockam_node::{Context, WorkerBuilder};
use ockam_vault::VaultForEncryptionAtRest;
use rand::Rng;
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct KafkaKeyExchangeListener {
    encryption_at_rest: Arc<dyn VaultForEncryptionAtRest>,
    rekey_period: Duration,
    key_validity: Duration,
    key_rotation: Duration,
}

#[derive(Debug, CborLen, Encode, Decode)]
#[rustfmt::skip]
pub(crate) struct KeyExchangeRequest {
}

#[derive(Debug, CborLen, Encode, Decode)]
#[rustfmt::skip]
pub(crate) struct KeyExchangeResponse {
    #[n(0)] pub key_identifier_for_consumer: Vec<u8>,
    #[n(1)] pub secret_key: [u8; 32],
    #[n(2)] pub valid_until: TimestampInSeconds,
    #[n(3)] pub rotate_after: TimestampInSeconds,
    #[n(4)] pub rekey_period: Duration,
}

impl Encodable for KeyExchangeRequest {
    fn encode(self) -> ockam_core::Result<Encoded> {
        ockam_core::cbor_encode_preallocate(self)
    }
}

impl Decodable for KeyExchangeRequest {
    fn decode(data: &[u8]) -> ockam_core::Result<Self> {
        Ok(minicbor::decode(data)?)
    }
}
impl Message for KeyExchangeRequest {}
impl Encodable for KeyExchangeResponse {
    fn encode(self) -> ockam_core::Result<Encoded> {
        ockam_core::cbor_encode_preallocate(self)
    }
}

impl Decodable for KeyExchangeResponse {
    fn decode(data: &[u8]) -> ockam_core::Result<Self> {
        Ok(minicbor::decode(data)?)
    }
}

impl Message for KeyExchangeResponse {}

#[async_trait]
impl Worker for KafkaKeyExchangeListener {
    type Context = Context;
    type Message = KeyExchangeRequest;

    async fn handle_message(
        &mut self,
        context: &mut Self::Context,
        message: Routed<Self::Message>,
    ) -> ockam_core::Result<()> {
        let mut secret_key = [0u8; 32];
        rand::thread_rng().fill(&mut secret_key[..]);
        let handle = self
            .encryption_at_rest
            .import_aead_key(secret_key.to_vec())
            .await?;

        let now = TimestampInSeconds(ockam_core::compat::time::now()?);
        let valid_until = now + self.key_validity;
        let rotate_after = now + self.key_rotation;

        context
            .send(
                message.return_route().clone(),
                KeyExchangeResponse {
                    key_identifier_for_consumer: handle.into_vec(),
                    secret_key,
                    valid_until,
                    rotate_after,
                    rekey_period: self.rekey_period,
                },
            )
            .await?;

        Ok(())
    }
}

impl KafkaKeyExchangeListener {
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        context: &Context,
        encryption_at_rest: Arc<dyn VaultForEncryptionAtRest>,
        key_rotation: Duration,
        key_validity: Duration,
        rekey_period: Duration,
        secure_channel_flow_control: &FlowControlId,
        incoming_access_control: impl IncomingAccessControl,
        outgoing_access_control: impl OutgoingAccessControl,
    ) -> ockam_core::Result<()> {
        let address = Address::from_string(DefaultAddress::KAFKA_CUSTODIAN);
        context
            .flow_controls()
            .add_consumer(address.clone(), secure_channel_flow_control);

        WorkerBuilder::new(KafkaKeyExchangeListener {
            encryption_at_rest,
            key_rotation,
            key_validity,
            rekey_period,
        })
        .with_address(address)
        .with_incoming_access_control(incoming_access_control)
        .with_outgoing_access_control(outgoing_access_control)
        .start(context)
        .await
    }
}
