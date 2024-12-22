use crate::channel_types::OneshotReceiver;
use crate::{relay::CtrlSignal, tokio::runtime::Handle, Context};
use ockam_core::{Processor, Result};

pub struct ProcessorRelay<P>
where
    P: Processor<Context = Context>,
{
    processor: P,
    ctx: Context,
}

impl<P> ProcessorRelay<P>
where
    P: Processor<Context = Context>,
{
    pub fn new(processor: P, ctx: Context) -> Self {
        Self { processor, ctx }
    }

    #[cfg_attr(not(feature = "std"), allow(unused_mut))]
    #[cfg_attr(not(feature = "std"), allow(unused_variables))]
    async fn run(self, ctrl_rx: OneshotReceiver<CtrlSignal>) {
        let mut ctx = self.ctx;
        let mut processor = self.processor;
        let ctx_addr = ctx.primary_address().clone();

        match processor.initialize(&mut ctx).await {
            Ok(()) => {}
            Err(e) => {
                error!(
                    "Failure during '{}' processor initialisation: {}",
                    ctx.primary_address(),
                    e
                );
                shutdown_and_stop_ack(&mut processor, &mut ctx, &ctx_addr).await;
                return;
            }
        }

        // This future encodes the main processor run loop logic
        let run_loop = async {
            loop {
                match processor.process(&mut ctx).await {
                    Ok(should_continue) => {
                        if !should_continue {
                            break;
                        }
                    }
                    Err(e) => {
                        #[cfg(feature = "debugger")]
                        error!(
                            "Error encountered during '{}' processing: {:?}",
                            ctx_addr, e
                        );
                        #[cfg(not(feature = "debugger"))]
                        error!("Error encountered during '{}' processing: {}", ctx_addr, e);
                    }
                }
            }

            Result::<()>::Ok(())
        };

        #[cfg(feature = "std")]
        {
            // Select over the two futures
            tokio::select! {
                // This future resolves when a stop control signal is received
                _ = ctrl_rx => {
                    debug!("Shutting down processor {} due to shutdown signal", ctx_addr);
                },
                _ = run_loop => {}
            };
        }

        // TODO wait on run_loop until we have a no_std select! implementation
        #[cfg(not(feature = "std"))]
        match run_loop.await {
            Ok(_) => trace!("Processor shut down cleanly {}", ctx_addr),
            Err(err) => error!("processor run loop aborted with error: {:?}", err),
        };

        // If we reach this point the router has signaled us to shut down
        shutdown_and_stop_ack(&mut processor, &mut ctx, &ctx_addr).await;
    }

    /// Create a processor relay with two node contexts
    pub(crate) fn init(
        rt: &Handle,
        processor: P,
        ctx: Context,
        ctrl_rx: OneshotReceiver<CtrlSignal>,
    ) {
        let relay = ProcessorRelay::<P>::new(processor, ctx);
        rt.spawn(relay.run(ctrl_rx));
    }
}

async fn shutdown_and_stop_ack<P>(
    processor: &mut P,
    ctx: &mut Context,
    ctx_addr: &ockam_core::Address,
) where
    P: Processor<Context = Context>,
{
    match processor.shutdown(ctx).await {
        Ok(()) => {}
        Err(e) => {
            error!("Failure during '{}' processor shutdown: {}", ctx_addr, e);
        }
    }

    // Finally send the router a stop ACK -- log errors
    trace!("Sending shutdown ACK");
    ctx.stop_ack().unwrap_or_else(|e| {
        error!("Failed to send stop ACK for '{}': {}", ctx_addr, e);
    });
}
