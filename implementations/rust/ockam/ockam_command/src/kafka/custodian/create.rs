use crate::node::util::initialize_default_node;
use crate::util::parsers::duration_parser;
use crate::{kafka::kafka_custodian_default_addr, node::NodeOpts, Command, CommandGlobalOpts};
use async_trait::async_trait;
use clap::{command, Args};
use colorful::Colorful;
use miette::miette;
use ockam_abac::PolicyExpression;
use ockam_api::colors::color_primary;
use ockam_api::nodes::models::services::{StartKafkaCustodianRequest, StartServiceRequest};
use ockam_api::nodes::BackgroundNodeClient;
use ockam_api::output::Output;
use ockam_api::{fmt_ok, DefaultAddress};
use ockam_core::api::Request;
use ockam_node::Context;
use serde::Serialize;
use std::fmt::Write;
use std::time::Duration;

/// Create a new Kafka Custodian.
#[derive(Clone, Debug, Args)]
pub struct CreateCommand {
    #[command(flatten)]
    pub node_opts: NodeOpts,

    /// The local address of the service
    #[arg(long, default_value_t = kafka_custodian_default_addr())]
    pub addr: String,

    /// Policy expression that will be used for access control to the Kafka Producer.
    /// If you don't provide it, the policy set for the "kafka-producer" resource type will be used.
    ///
    /// You can check the fallback policy with `ockam policy show --resource-type kafka-producer`.
    #[arg(hide = true, long = "allow-producer", id = "PRODUCER-EXPRESSION")]
    pub producer_policy_expression: Option<PolicyExpression>,

    #[arg(long, default_value = "24h", value_name = "DURATION", value_parser = duration_parser)]
    pub key_rotation: Duration,

    #[arg(long, default_value = "30h", value_name = "DURATION", value_parser = duration_parser)]
    pub key_validity: Duration,

    #[arg(long, default_value = "1h", value_name = "DURATION", value_parser = duration_parser)]
    pub rekey_period: Duration,

    /// The name of the Vault used to decrypt the encrypted data.
    /// When not provided, the default Vault will be used.
    #[arg(long, value_name = "VAULT_NAME")]
    pub vault: Option<String>,
}

#[async_trait]
impl Command for CreateCommand {
    const NAME: &'static str = "kafka-custodian create";

    async fn async_run(self, ctx: &Context, opts: CommandGlobalOpts) -> crate::Result<()> {
        if self.key_validity < self.key_rotation {
            return Err(miette!("Key validity must be greater than key rotation"));
        }

        if self.rekey_period.is_zero() {
            return Err(miette!("Rekey period must be greater than zero"));
        }

        if self.rekey_period >= self.key_rotation {
            return Err(miette!("Rekey period must be less than key rotation"));
        }

        initialize_default_node(ctx, &opts).await?;

        let at_node = self.node_opts.at_node.clone();
        let addr = self.addr.clone();

        let output = {
            let pb = opts.terminal.spinner();
            if let Some(pb) = pb.as_ref() {
                pb.set_message("Creating Kafka Custodian...\n".to_string());
            }

            let node = BackgroundNodeClient::create(ctx, &opts.state, &at_node).await?;

            let payload = StartKafkaCustodianRequest {
                vault: self.vault.clone(),
                producer_policy_expression: self.producer_policy_expression,
                key_rotation: self.key_rotation,
                key_validity: self.key_validity,
                rekey_period: self.rekey_period,
            };
            let payload = StartServiceRequest::new(payload, &addr);
            let req = Request::post(format!(
                "/node/services/{}",
                DefaultAddress::KAFKA_CUSTODIAN
            ))
            .body(payload);
            node.tell(ctx, req)
                .await
                .map_err(|e| miette!("Failed to start Kafka Custodian: {e}"))?;

            KafkaCustodianOutput {
                node_name: node.node_name(),
            }
        };

        opts.terminal
            .stdout()
            .plain(output.item()?)
            .json_obj(output)?
            .write_line()?;

        Ok(())
    }
}

#[derive(Serialize)]
struct KafkaCustodianOutput {
    node_name: String,
}

impl Output for KafkaCustodianOutput {
    fn item(&self) -> ockam_api::Result<String> {
        let mut f = String::new();
        writeln!(
            f,
            "{}\n",
            fmt_ok!(
                "Created a new Kafka Custodian in the Node {}",
                color_primary(&self.node_name),
            )
        )?;

        Ok(f)
    }
}
