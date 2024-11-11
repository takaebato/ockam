use colorful::Colorful;
use miette::{miette, IntoDiagnostic};
use tracing::debug;

use ockam::Context;
use ockam_api::colors::color_primary;
use ockam_api::fmt_warn;
use ockam_api::nodes::BackgroundNodeClient;

use crate::node::show::get_node_resources;
use crate::node::util::spawn_node;
use crate::node::CreateCommand;
use crate::CommandGlobalOpts;

impl CreateCommand {
    // Create a new node running in the background (i.e. another, new OS process)
    pub(crate) async fn background_mode(
        &self,
        ctx: &Context,
        opts: CommandGlobalOpts,
    ) -> miette::Result<()> {
        let node_name = self.name.clone();
        debug!(%node_name, "creating node in background mode");

        // Early checks
        if self.foreground_args.child_process {
            return Err(miette!(
                "Cannot create a background node from another background node"
            ));
        }
        if opts
            .state
            .get_node(&node_name)
            .await
            .ok()
            .map(|n| n.is_running())
            .unwrap_or(false)
        {
            return Err(miette!(
                "Node {} has been already created",
                color_primary(&node_name)
            ));
        }
        self.get_or_create_identity(&opts, &self.identity).await?;
        self.clone().spawn_background_node(&opts).await?;
        let mut node = BackgroundNodeClient::create_to_node(ctx, &opts.state, &node_name).await?;
        let node_resources = get_node_resources(ctx, &opts.state, &mut node, true).await?;

        // Output
        if !node_resources.status.is_running() {
            opts.terminal.write_line(fmt_warn!(
                "Node was {} created but is not reachable",
                color_primary(&node_name)
            ))?;
        }

        opts.terminal
            .clone()
            .stdout()
            .plain(self.plain_output(&opts, &node_name).await?)
            .machine(&node_name)
            .json(serde_json::to_string(&node_resources).into_diagnostic()?)
            .write_line()?;

        Ok(())
    }

    pub(crate) async fn spawn_background_node(
        self,
        opts: &CommandGlobalOpts,
    ) -> miette::Result<()> {
        spawn_node(opts, self).await
    }
}
