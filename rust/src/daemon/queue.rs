use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread;

type Operation = Box<dyn FnOnce() + Send>;

/// One queue of tuning writes per GPU, each run in order on its own worker thread, so a slow
/// GPU never holds up another. Workers start on a GPU's first write and stop when the queues
/// are dropped.
#[derive(Default)]
pub(super) struct GpuOperationQueues {
    workers: Mutex<HashMap<String, Sender<Operation>>>,
}

impl GpuOperationQueues {
    pub fn submit<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> PendingOperation<T> {
        let (result, pending) = mpsc::channel();
        let operation: Operation = Box::new(move || {
            let _ = result.send(operation());
        });
        self.workers
            .lock()
            .unwrap()
            .entry(gpu_id.to_string())
            .or_insert_with(spawn_worker)
            .send(operation)
            .expect("GPU operation queue worker stopped");
        PendingOperation(pending)
    }
}

fn spawn_worker() -> Sender<Operation> {
    let (sender, operations) = mpsc::channel::<Operation>();
    thread::Builder::new()
        .name("prismd-gpu-queue".into())
        .spawn(move || {
            for operation in operations {
                // A panicking operation fails only its own request; the queue keeps serving.
                let _ = panic::catch_unwind(AssertUnwindSafe(operation));
            }
        })
        .expect("failed to start a GPU operation queue worker");
    sender
}

pub(super) struct PendingOperation<T>(Receiver<T>);

impl<T> PendingOperation<T> {
    pub fn wait(self) -> T {
        self.0.recv().expect("GPU operation panicked")
    }
}
