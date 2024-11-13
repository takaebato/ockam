use super::Router;
use alloc::sync::Arc;
use alloc::vec::Vec;
use ockam_core::{Address, Result};

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
        let mut receiver = if let Some(receiver) = receiver {
            receiver
        } else {
            debug!("Node is already terminated");
            return Ok(());
        };

        // Start by shutting down clusterless workers
        let cluster = self.map.stop_all_non_cluster_workers().await;

        // If there _are_ no clusterless workers we go to the next cluster
        if cluster.is_empty() {
            let result = self.stop_next_cluster().await;
            if let Ok(true) = result {
                self.state.terminate().await;
                return Ok(());
            }
        }

        // Otherwise: keep track of addresses we are stopping
        cluster
            .into_iter()
            .for_each(|addr| self.map.init_stop(addr));

        // Start a timeout task to interrupt us...
        #[cfg(feature = "std")]
        {
            use core::time::Duration;
            use tokio::{task, time};

            let dur = Duration::from_secs(seconds as u64);
            task::spawn(async move {
                time::sleep(dur).await;
                warn!("Shutdown timeout reached; aborting node!");
                self.map.clear_address_records_map();
                self.state.terminate().await;
            });
        }

        receiver.recv().await;
        Ok(())
    }

    pub(crate) async fn stop_next_cluster(&self) -> Result<bool> {
        let next_cluster_addresses = self.map.next_cluster();

        match next_cluster_addresses {
            Some(vec) => {
                self.stop_cluster_addresses(vec).await?;
                Ok(false)
            }
            // If not, we are done!
            None => Ok(true),
        }
    }

    async fn stop_cluster_addresses(&self, addresses: Vec<Address>) -> Result<()> {
        for address in addresses.iter() {
            self.map.stop(address).await?;
        }
        Ok(())
    }
}
