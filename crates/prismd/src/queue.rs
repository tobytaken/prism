use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::Duration;

const IDLE_WORKER_LIFETIME: Duration = Duration::from_secs(30);

type Operation = Box<dyn FnOnce() + Send>;
type Workers = Mutex<HashMap<String, Sender<Operation>>>;

/// One queue of tuning writes per GPU, each run in order on its own worker thread, so a slow
/// GPU never holds up another. A GPU's worker starts with its first write, stops after a
/// while without writes, and stops when the queues are dropped.
pub(super) struct GpuOperationQueues {
    workers: Arc<Workers>,
    idle_lifetime: Duration,
}

impl Default for GpuOperationQueues {
    fn default() -> Self {
        Self::with_idle_lifetime(IDLE_WORKER_LIFETIME)
    }
}

impl GpuOperationQueues {
    /// Waits for already queued work, including work for GPUs that have left the inventory.
    #[cfg(test)]
    pub fn settle(&self) {
        let ids: Vec<_> = lock(&self.workers).keys().cloned().collect();
        for id in ids {
            self.submit(&id, || ()).wait();
        }
    }

    fn with_idle_lifetime(idle_lifetime: Duration) -> Self {
        Self {
            workers: Arc::default(),
            idle_lifetime,
        }
    }

    pub fn submit<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> PendingOperation<T> {
        let (result, pending) = mpsc::channel();
        let operation: Operation = Box::new(move || {
            let _ = result.send(operation());
        });
        let mut workers = lock(&self.workers);
        let worker = workers.entry(gpu_id.to_string()).or_insert_with(|| {
            spawn_worker(
                gpu_id.to_string(),
                Arc::downgrade(&self.workers),
                self.idle_lifetime,
            )
        });
        // A worker only stops after removing itself under this lock, so it is still receiving.
        let _ = worker.send(operation);
        PendingOperation(pending)
    }
}

fn spawn_worker(
    gpu_id: String,
    workers: Weak<Workers>,
    idle_lifetime: Duration,
) -> Sender<Operation> {
    let (sender, operations) = mpsc::channel::<Operation>();
    thread::Builder::new()
        .name("prismd-gpu-queue".into())
        .spawn(move || loop {
            let operation = match operations.recv_timeout(idle_lifetime) {
                Ok(operation) => operation,
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    let Some(workers) = workers.upgrade() else {
                        return;
                    };
                    // Holding the lock, no new operation can arrive between this check and removal.
                    let mut workers = lock(&workers);
                    match operations.try_recv() {
                        Ok(operation) => operation,
                        Err(TryRecvError::Empty) => {
                            workers.remove(&gpu_id);
                            return;
                        }
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
            };
            // A panicking operation fails only its own request; the queue keeps serving.
            let _ = panic::catch_unwind(AssertUnwindSafe(operation));
        })
        .expect("failed to start a GPU operation queue worker");
    sender
}

// No operation runs under this lock, so a panic elsewhere never leaves the map inconsistent.
fn lock(workers: &Workers) -> MutexGuard<'_, HashMap<String, Sender<Operation>>> {
    workers.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) struct PendingOperation<T>(Receiver<T>);

impl<T> PendingOperation<T> {
    pub fn wait(self) -> T {
        self.0.recv().expect("GPU operation panicked")
    }

    /// The result, unless the operation is still queued or running when the timeout ends.
    pub fn wait_timeout(self, timeout: Duration) -> Option<T> {
        self.0.recv_timeout(timeout).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::GpuOperationQueues;
    use std::time::Duration;

    #[test]
    fn a_gpu_queue_keeps_serving_after_its_worker_has_been_idle() {
        let queues = GpuOperationQueues::with_idle_lifetime(Duration::from_millis(10));
        assert_eq!(queues.submit("GPU-123", || 1).wait(), 1);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(queues.submit("GPU-123", || 2).wait(), 2);
    }

    #[test]
    fn a_panicking_write_fails_only_its_own_request() {
        let queues = GpuOperationQueues::default();
        let failed = queues.submit("GPU-123", || -> u32 { panic!("setter panicked") });
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| failed.wait())).is_err());
        assert_eq!(queues.submit("GPU-123", || 2).wait(), 2);
    }
}
