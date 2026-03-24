use astrcode_core::Result;

use crate::PluginProcess;

#[derive(Default)]
pub struct LifecycleManager {
    processes: Vec<PluginProcess>,
}

impl LifecycleManager {
    pub fn register(&mut self, process: PluginProcess) {
        self.processes.push(process);
    }

    pub async fn shutdown_all(&mut self) -> Result<()> {
        for process in &mut self.processes {
            process.shutdown().await?;
        }
        Ok(())
    }
}
