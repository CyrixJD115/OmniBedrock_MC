use crate::server::{ServerManager, Status, now};
use omnibedrock_core::config::Config;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Metrics {
    pub latest: watch::Receiver<Value>,
}
impl Metrics {
    pub fn start(
        manager: ServerManager,
        _config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> Self {
        let (tx, latest) = watch::channel(
            json!({"timestamp":now(),"status":"stopped","cpu_percent":0.0,"memory_mb":0.0,"memory_percent":0.0,"tps":0.0}),
        );
        tokio::spawn(async move {
            let mut system = System::new_with_specifics(RefreshKind::nothing());
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _=shutdown.cancelled()=>break, _=ticker.tick()=>{} }
                let snapshot = manager.snapshot();
                let result = tokio::task::spawn_blocking(move || {
                    system.refresh_memory();
                    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cpu().with_memory());
                    let mut cpu = 0.0; let mut memory = 0u64;
                    if let Some(pid) = snapshot.pid {
                        // Endstone is a launcher: include descendants, not just the Python parent.
                        let mut pids = vec![Pid::from_u32(pid)]; let mut cursor=0;
                        while cursor<pids.len() {
                            let parent=pids[cursor]; cursor+=1;
                            for (pid, process) in system.processes() { if process.parent()==Some(parent) && !pids.contains(pid) {pids.push(*pid);} }
                        }
                        for pid in pids {if let Some(p) = system.process(pid) {cpu+=p.cpu_usage(); memory+=p.memory();}}
                    }
                    let total=system.total_memory();
                    let data=json!({"timestamp":now(),"status":snapshot.status,"cpu_percent":cpu,"memory_mb":memory as f64/1048576.0,"memory_percent":if total>0 {100.0*memory as f64/total as f64}else{0.0},"tps":if snapshot.status==Status::Running {snapshot.tps.unwrap_or(20.0)}else{0.0},"host_memory_mb":total as f64/1048576.0});
                    (system,data)
                }).await;
                match result {
                    Ok((s, data)) => {
                        system = s;
                        tx.send_replace(data);
                    }
                    Err(e) => {
                        tracing::error!(%e,"metrics task failed");
                        break;
                    }
                }
            }
        });
        Self { latest }
    }
}
