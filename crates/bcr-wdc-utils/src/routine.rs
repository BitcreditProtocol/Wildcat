// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
// ----- local imports

// ----- end imports

pub type TStamp = chrono::DateTime<chrono::Utc>;

#[async_trait]
pub trait Routine: Send + 'static {
    // the routine can optionally return the next sleep time
    async fn run_task(&self, now: TStamp) -> AnyResult<Option<std::time::Duration>>;
}

pub struct RoutineHandle {
    pub cancel: tokio_util::sync::CancellationToken,
    pub task: tokio::task::JoinHandle<()>,
}

async fn run_routine<R: Routine>(routine: R, cancel: CancellationToken, mut interval: Duration) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(interval) => {
                let now = chrono::Utc::now();
                interval = match routine.run_task(now).await {
                    Ok(None) => {tracing::info!("Routine checks completed successfully");
                        interval},
                        Ok(Some(next_interval)) => {
                            tracing::info!("Routine checks completed successfully, next check in {next_interval:?}");
                            next_interval
                        },
                    Err(e) => {tracing::error!("Routine checks failed: {e}");
                        interval
                    },
                }
            }
        }
    }
}

impl RoutineHandle {
    pub fn new<R: Routine>(routine: R, interval: std::time::Duration) -> Self {
        let cancel = tokio_util::sync::CancellationToken::new();
        let task = tokio::task::spawn(run_routine(routine, cancel.clone(), interval));
        Self { cancel, task }
    }

    pub async fn stop(self) {
        self.cancel.cancel();
        match self.task.await {
            Ok(()) => tracing::info!("Routine stopped successfully"),
            Err(e) => tracing::error!("Failed to stop routine: {e}"),
        }
    }
}
