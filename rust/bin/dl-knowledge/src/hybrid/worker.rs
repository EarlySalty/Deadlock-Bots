use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use tokio::sync::Semaphore;

pub struct Worker {
    gate: Arc<Semaphore>,
    budget: Duration,
}

impl Worker {
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            gate: Arc::new(Semaphore::new(1)),
            budget: Duration::from_millis(timeout_ms),
        }
    }

    pub async fn run<T, F>(&self, job: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Instant) -> Result<T> + Send + 'static,
    {
        let permit = self
            .gate
            .clone()
            .try_acquire_owned()
            .context("Hybrid-Worker belegt")?;
        let deadline = Instant::now() + self.budget;
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            ensure!(
                Instant::now() < deadline,
                "Hybrid-Zeitbudget vor Start überschritten"
            );
            let result = job(deadline)?;
            ensure!(Instant::now() < deadline, "Hybrid-Zeitbudget überschritten");
            Ok(result)
        });
        tokio::time::timeout(self.budget, task)
            .await
            .context("Hybrid-Zeitbudget überschritten")?
            .context("Hybrid-Worker abgebrochen")?
    }
}
