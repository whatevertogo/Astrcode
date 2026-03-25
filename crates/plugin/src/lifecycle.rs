use astrcode_core::Result;

use crate::Supervisor;

#[derive(Default)]
pub struct LifecycleManager {
    supervisors: Vec<Supervisor>,
}

impl LifecycleManager {
    pub fn register(&mut self, supervisor: Supervisor) {
        self.supervisors.push(supervisor);
    }

    pub async fn shutdown_all(&self) -> Result<()> {
        for supervisor in &self.supervisors {
            supervisor.shutdown().await?;
        }
        Ok(())
    }
}
