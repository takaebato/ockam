use super::Router;
use ockam_core::Result;
use std::sync::Arc;

/// Register a stop ACK
///
/// For every ACK we re-test whether the current cluster has stopped.
/// If not, we do nothing. If so, we trigger the next cluster to stop.

impl Router {
    /// Implement the graceful shutdown strategy
    #[cfg_attr(not(feature = "std"), allow(unused_variables))]
    pub async fn stop_graceful(self: Arc<Router>, seconds: u8) -> Result<()> {
        // Mark the router as shutting down to prevent spawning
        info!("Initiate graceful node shutdown");
        // This changes the router state to `Stopping`
        let receiver = self.state.shutdown();
        let receiver = if let Some(receiver) = receiver {
            receiver
        } else {
            debug!("Node is already terminated");
            return Ok(());
        };

        // Start by shutting down clusterless workers
        let no_non_cluster_workers = self.map.stop_all_non_cluster_workers();

        // Not stop cluster addresses
        let no_cluster_workers = self.map.stop_all_cluster_workers();

        if no_non_cluster_workers && no_cluster_workers {
            // No stop ack will arrive because we didn't stop anything
            return Ok(());
        }

        // Start a timeout task to interrupt us...
        #[cfg(feature = "std")]
        {
            use core::time::Duration;
            use tokio::{task, time};

            let dur = Duration::from_secs(seconds as u64);
            task::spawn(async move {
                time::sleep(dur).await;
                warn!("Shutdown timeout reached; aborting node!");
                let uncleared_addresses = self.map.force_clear_records();

                if !uncleared_addresses.is_empty() {
                    error!("Router internal inconsistency detected. Records map is not empty after stopping all workers. Addresses: {:?}", uncleared_addresses);
                }

                self.state.terminate();
            });
        }

        receiver.await.unwrap(); //FIXME

        Ok(())
    }
}
