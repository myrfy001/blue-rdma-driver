use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use log::{error, info};

pub(crate) trait SingleThreadTaskWorker {
    type Task;

    fn process(&mut self, task: Self::Task);

    fn spawn(mut self, name: &str, abort: AbortSignal) -> TaskTx<Self::Task>
    where
        Self: Sized + Send + 'static,
        Self::Task: Send + 'static,
    {
        let name = name.to_owned();
        let (tx, rx) = flume::unbounded::<Self::Task>();
        let abort = AbortSignal::new();
        let abort_c = abort.clone();
        let _handle = std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || {
                info!("worker {name} running");
                loop {
                    if abort.should_abort() {
                        break;
                    }
                    if let Ok(task) = rx.recv() {
                        self.process(task);
                    } else {
                        error!("failed to recv task from channel");
                        break;
                    }
                }
                info!("worker {name} exited");
            })
            .expect("failed to spawn worker");

        TaskTx { inner: tx }
    }

    fn spawn_polling(
        mut self,
        name: &str,
        abort: AbortSignal,
        interval: Duration,
    ) -> TaskTx<Self::Task>
    where
        Self: Sized + Send + 'static,
        Self::Task: Send + 'static,
    {
        let name = name.to_owned();
        let (tx, rx) = flume::unbounded::<Self::Task>();
        let abort = AbortSignal::new();
        let abort_c = abort.clone();
        let _handle = std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || {
                info!("worker {name} running");
                loop {
                    spin_sleep::sleep(interval);
                    if abort.should_abort() {
                        break;
                    }
                    for task in rx.try_iter() {
                        self.process(task);
                    }
                }
                info!("worker {name} exited");
            })
            .expect("failed to spawn worker");

        TaskTx { inner: tx }
    }
}

pub(crate) struct TaskTx<T> {
    inner: flume::Sender<T>,
}

impl<T> TaskTx<T> {
    pub(crate) fn send(&self, task: T) {
        self.inner
            .send(task)
            .expect("failed to send task to channel");
    }
}

impl<T> Clone for TaskTx<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AbortSignal {
    inner: Arc<AtomicBool>,
}

impl AbortSignal {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(true)),
        }
    }

    fn should_abort(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }

    pub(crate) fn abort(&self) {
        self.inner.store(false, Ordering::Relaxed);
    }
}

