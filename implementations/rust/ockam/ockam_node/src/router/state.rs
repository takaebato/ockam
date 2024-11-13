//! Router run state utilities
use crate::channel_types::{SmallReceiver, SmallSender};
use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::AtomicBool;
use ockam_core::compat::sync::Mutex as SyncMutex;

/// Node state
#[derive(Clone)]
pub enum NodeState {
    Running,
    Stopping,
    Terminated,
}

pub struct RouterState {
    node_state: SyncMutex<NodeState>,
    termination_senders: SyncMutex<Vec<SmallSender<()>>>,
    is_running: AtomicBool,
}

impl RouterState {
    pub fn new() -> Self {
        Self {
            node_state: SyncMutex::new(NodeState::Running),
            termination_senders: SyncMutex::new(Default::default()),
            is_running: AtomicBool::new(true),
        }
    }

    /// Set the router state to `Stopping` and return a receiver
    /// to wait for the shutdown to complete.
    /// When `None` is returned, the router is already terminated.
    pub(super) fn shutdown(&self) -> Option<SmallReceiver<()>> {
        let mut guard = self.node_state.lock().unwrap();
        match guard.deref_mut() {
            NodeState::Running => {
                let (sender, receiver) = crate::channel_types::small_channel();
                *guard = NodeState::Stopping;
                self.is_running
                    .store(false, core::sync::atomic::Ordering::Relaxed);
                self.termination_senders.lock().unwrap().push(sender);
                Some(receiver)
            }
            NodeState::Stopping => {
                let (sender, receiver) = crate::channel_types::small_channel();
                self.termination_senders.lock().unwrap().push(sender);
                Some(receiver)
            }
            NodeState::Terminated => None,
        }
    }

    /// Set the router to `Terminated` state and notify all tasks waiting for shutdown
    pub(super) async fn terminate(&self) {
        self.is_running
            .store(false, core::sync::atomic::Ordering::Relaxed);
        let previous = {
            let mut guard = self.node_state.lock().unwrap();
            core::mem::replace(guard.deref_mut(), NodeState::Terminated)
        };

        match previous {
            NodeState::Running | NodeState::Stopping => {
                info!("No more workers left. Goodbye!");
                let senders = {
                    let mut guard = self.termination_senders.lock().unwrap();
                    core::mem::take(guard.deref_mut())
                };
                for sender in senders {
                    let _ = sender.send(()).await;
                }
            }
            NodeState::Terminated => {}
        }
    }

    pub(super) async fn wait_termination(&self) {
        let mut receiver = {
            let guard = self.node_state.lock().unwrap();
            match guard.deref() {
                NodeState::Running | NodeState::Stopping => {
                    let (sender, receiver) = crate::channel_types::small_channel();
                    self.termination_senders.lock().unwrap().push(sender);
                    receiver
                }
                NodeState::Terminated => {
                    return;
                }
            }
        };
        receiver.recv().await;
    }

    pub(super) fn running(&self) -> bool {
        self.is_running.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// Check if this router is still `running`, meaning allows
    /// spawning new workers and processors
    pub(super) fn node_state(&self) -> NodeState {
        self.node_state.lock().unwrap().clone()
    }
}
