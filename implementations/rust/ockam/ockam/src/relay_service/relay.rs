use crate::Context;
use ockam_core::compat::sync::Arc;
use ockam_core::compat::{boxed::Box, vec::Vec};
use ockam_core::{
    route, Address, AllowAll, AllowOnwardAddress, Any, IncomingAccessControl, LocalMessage,
    OutgoingAccessControl, Result, Route, Routed, Worker,
};
use ockam_node::WorkerBuilder;
use tracing::info;

pub(super) struct Relay {
    forward_route: Route,
    // this option will be `None` after this worker is initialized, because
    // while initializing, the worker will send the payload contained in this
    // field to the `forward_route`, to indicate a successful connection
    payload: Option<Vec<u8>>,
}

impl Relay {
    pub(super) async fn create(
        ctx: &Context,
        address: Address,
        forward_route: Route,
        registration_payload: Vec<u8>,
        incoming_access_control: Arc<dyn IncomingAccessControl>,
    ) -> Result<()> {
        info!("Created new alias {} for {}", address, forward_route);

        // Should be able to reach last and second last hops
        let outgoing_access_control: Arc<dyn OutgoingAccessControl> = if forward_route.len() == 1 {
            // We are accessed with our node, no transport is involved
            Arc::new(AllowAll)
        } else {
            let next_hop = forward_route.next()?.clone();
            Arc::new(AllowOnwardAddress(next_hop))
        };

        let relay = Self {
            forward_route,
            payload: Some(registration_payload.clone()),
        };

        WorkerBuilder::new(relay)
            .with_address(address)
            .with_incoming_access_control_arc(incoming_access_control)
            .with_outgoing_access_control_arc(outgoing_access_control)
            .start(ctx)
            .await?;

        Ok(())
    }
}

#[crate::worker]
impl Worker for Relay {
    type Context = Context;
    type Message = Any;

    async fn initialize(&mut self, ctx: &mut Self::Context) -> Result<()> {
        let payload = self
            .payload
            .take()
            .expect("payload must be available on init");

        ctx.forward(
            LocalMessage::new()
                .with_onward_route(self.forward_route.clone())
                .with_return_route(route![ctx.primary_address()])
                .with_payload(payload),
        )
        .await?;

        // Remove the last hop so that just route to the node itself is left
        self.forward_route = self.forward_route.clone().modify().pop_back().into();

        Ok(())
    }

    async fn handle_message(
        &mut self,
        ctx: &mut Self::Context,
        msg: Routed<Self::Message>,
    ) -> Result<()> {
        let mut local_message = msg.into_local_message();

        local_message = local_message
            .pop_front_onward_route()?
            .prepend_front_onward_route(&self.forward_route);

        let next_hop = local_message.next_on_onward_route()?;
        let prev_hop = local_message.return_route().next()?;

        if let Some(info) = ctx
            .flow_controls()
            .find_flow_control_with_producer_address(&next_hop)
        {
            ctx.flow_controls()
                .add_consumer(prev_hop.clone(), info.flow_control_id());
        }

        if let Some(info) = ctx
            .flow_controls()
            .find_flow_control_with_producer_address(prev_hop)
        {
            ctx.flow_controls()
                .add_consumer(next_hop.clone(), info.flow_control_id());
        }

        ctx.forward(local_message).await
    }
}
