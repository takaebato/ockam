use async_trait::async_trait;
use clap::Args;
use colorful::Colorful;
use console::Term;
use ockam_api::colors::color_primary;
use ockam_api::{fmt_ok, DefaultAddress};

use ockam_api::nodes::models::services::{DeleteServiceRequest, ServiceStatus};
use ockam_api::nodes::BackgroundNodeClient;
use ockam_api::terminal::{Terminal, TerminalStream};
use ockam_core::api::Request;
use ockam_core::AsyncTryClone;
use ockam_node::Context;

use crate::tui::{DeleteCommandTui, PluralTerm};
use crate::{docs, node::NodeOpts, Command, CommandGlobalOpts};

const AFTER_LONG_HELP: &str = include_str!("./static/delete/after_long_help.txt");

/// Delete a Kafka Custodian
#[derive(Clone, Debug, Args)]
#[command(after_long_help = docs::after_help(AFTER_LONG_HELP))]
pub struct DeleteCommand {
    #[command(flatten)]
    pub node_opts: NodeOpts,

    /// Kafka Custodian service address
    pub address: Option<String>,

    /// Confirm the deletion without prompting
    #[arg(display_order = 901, long, short)]
    pub(crate) yes: bool,

    /// Delete all the Kafka Custodians
    #[arg(long)]
    pub(crate) all: bool,
}

#[async_trait]
impl Command for DeleteCommand {
    const NAME: &'static str = "kafka-custodian delete";

    async fn async_run(self, ctx: &Context, opts: CommandGlobalOpts) -> crate::Result<()> {
        Ok(DeleteTui::run(ctx, opts, self).await?)
    }
}

#[derive(AsyncTryClone)]
struct DeleteTui {
    ctx: Context,
    opts: CommandGlobalOpts,
    node: BackgroundNodeClient,
    cmd: DeleteCommand,
}

impl DeleteTui {
    pub async fn run(
        ctx: &Context,
        opts: CommandGlobalOpts,
        cmd: DeleteCommand,
    ) -> miette::Result<()> {
        let node = BackgroundNodeClient::create(ctx, &opts.state, &cmd.node_opts.at_node).await?;
        let tui = Self {
            ctx: ctx.async_try_clone().await?,
            opts,
            node,
            cmd,
        };
        tui.delete().await
    }
}

#[async_trait]
impl DeleteCommandTui for DeleteTui {
    const ITEM_NAME: PluralTerm = PluralTerm::KafkaCustodian;

    fn cmd_arg_item_name(&self) -> Option<String> {
        self.cmd.address.clone()
    }

    fn cmd_arg_delete_all(&self) -> bool {
        self.cmd.all
    }

    fn cmd_arg_confirm_deletion(&self) -> bool {
        self.cmd.yes
    }

    fn terminal(&self) -> Terminal<TerminalStream<Term>> {
        self.opts.terminal.clone()
    }

    async fn list_items_names(&self) -> miette::Result<Vec<String>> {
        let instances: Vec<ServiceStatus> = self
            .node
            .ask(
                &self.ctx,
                Request::get(format!(
                    "/node/services/{}",
                    DefaultAddress::KAFKA_CUSTODIAN
                )),
            )
            .await?;
        let addresses = instances.into_iter().map(|i| i.addr).collect();
        Ok(addresses)
    }

    async fn delete_single(&self, item_name: &str) -> miette::Result<()> {
        self.node
            .tell(
                &self.ctx,
                Request::delete(format!(
                    "/node/services/{}",
                    DefaultAddress::KAFKA_CUSTODIAN
                ))
                .body(DeleteServiceRequest::new(item_name)),
            )
            .await?;
        let node_name = self.node.node_name();
        self.terminal()
            .stdout()
            .plain(fmt_ok!(
                "Kafka Custodian with address {} on Node {} has been deleted",
                color_primary(item_name),
                color_primary(&node_name)
            ))
            .json(serde_json::json!({ "address": item_name, "node": node_name }))
            .write_line()?;
        Ok(())
    }
}
