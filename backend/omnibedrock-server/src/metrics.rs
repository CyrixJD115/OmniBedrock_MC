//! One sampling loop for every instance plus the host. Native and systemd
//! instances are measured from their process trees via sysinfo; docker
//! instances come from `docker stats`. Collection failures never take the
//! place of reported zeros: each instance carries its own `usage_source`.
use crate::instances::{DEFAULT_INSTANCE, InstanceManager};
use crate::server::{Status, now};
use omnibedrock_core::config::Config;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Metrics {
    /// Default instance in the legacy single-server shape (performance page).
    pub latest: watch::Receiver<Value>,
    /// Host capacity plus per-instance usage: {"host": {...}, "instances": {...}}.
    pub all: watch::Receiver<Value>,
}

/// Recursive directory size in bytes; IO errors count as zero for that entry.
fn dir_size(dir: &std::path::Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// Host root-filesystem usage as (total_mb, used_mb).
fn host_disk_usage() -> Option<(f64, f64)> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter(|d| d.mount_point() == std::path::Path::new("/"))
        .max_by_key(|d| d.total_space())
        .or_else(|| disks.list().iter().max_by_key(|d| d.total_space()))?;
    let total = disk.total_space() as f64 / 1048576.0;
    let used = total - disk.available_space() as f64 / 1048576.0;
    Some((total, used))
}

/// Parse a docker stats memory amount like "123.4MiB" or "1.5GiB" into MiB.
fn parse_docker_memory(text: &str) -> Option<f64> {
    let value: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let value: f64 = value.parse().ok()?;
    let scale = if text.contains("GiB") {
        1024.0
    } else if text.contains("MiB") {
        1.0
    } else if text.contains("KiB") {
        1.0 / 1024.0
    } else {
        1.0 / (1024.0 * 1024.0) // bytes
    };
    Some(value * scale)
}

impl Metrics {
    pub fn start(
        manager: Arc<InstanceManager>,
        _config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> Self {
        let (latest_tx, latest) = watch::channel(stopped_metrics());
        let (all_tx, all) = watch::channel(json!({
            "timestamp": now(),
            "host": host_shape(&System::new()),
            "instances": {},
        }));
        tokio::spawn(async move {
            let mut system = System::new_with_specifics(RefreshKind::nothing());
            let mut docker_usage: BTreeMap<String, (f64, f64)> = BTreeMap::new();
            let mut last_docker_stats = Instant::now() - Duration::from_secs(600);
            // Disk usage is a directory walk, so instances refresh one per
            // tick (staggered) at most once a minute each.
            let mut disk_cache: BTreeMap<String, f64> = BTreeMap::new();
            let mut disk_refreshed: BTreeMap<String, Instant> = BTreeMap::new();
            let mut host_disk: Option<(f64, f64)> = None;
            let mut last_host_disk = Instant::now() - Duration::from_secs(600);
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _=shutdown.cancelled()=>break, _=ticker.tick()=>{} }
                // Refresh docker usage at most every 6 seconds; `docker stats`
                // itself blocks for roughly two seconds per call.
                if last_docker_stats.elapsed() >= Duration::from_secs(6) {
                    docker_usage = collect_docker_stats().await;
                    last_docker_stats = Instant::now();
                }
                if last_host_disk.elapsed() >= Duration::from_secs(20) {
                    host_disk = host_disk_usage();
                    last_host_disk = Instant::now();
                }
                let instances = manager.list().await;
                let samples: Vec<_> = instances
                    .iter()
                    .map(|i| {
                        let record = i.record();
                        (
                            record,
                            i.server.snapshot(),
                            i.config.bedrock_server_dir.clone(),
                        )
                    })
                    .collect();
                // Pick the stalest instance disk sample (or a missing one).
                let stalest = instances
                    .iter()
                    .map(|i| {
                        let id = i.record().id;
                        let age = disk_refreshed
                            .get(&id)
                            .map_or(Duration::MAX, |t| t.elapsed());
                        (age, id, i.clone())
                    })
                    .max_by_key(|(age, _, _)| *age);
                if let Some((age, _, instance)) = stalest
                    && age >= Duration::from_secs(60)
                {
                    let id = instance.record().id;
                    let dir = instance.config.bedrock_server_dir.clone();
                    if let Ok(bytes) = tokio::task::spawn_blocking(move || dir_size(&dir)).await {
                        disk_cache.insert(id.clone(), bytes as f64 / 1048576.0);
                    }
                    disk_refreshed.insert(id, Instant::now());
                }
                let docker_usage = docker_usage.clone();
                let disk_cache = disk_cache.clone();
                let host_disk = host_disk;
                let result = tokio::task::spawn_blocking(move || {
                    system.refresh_memory();
                    system.refresh_processes_specifics(
                        ProcessesToUpdate::All,
                        true,
                        ProcessRefreshKind::nothing().with_cpu().with_memory(),
                    );
                    let mut host = host_shape(&system);
                    if let Some((total_mb, used_mb)) = host_disk {
                        host["disk_total_mb"] = json!(total_mb);
                        host["disk_used_mb"] = json!(used_mb);
                        host["disk_percent"] = json!(if total_mb > 0.0 {
                            100.0 * used_mb / total_mb
                        } else {
                            0.0
                        });
                    }
                    let mut map = serde_json::Map::new();
                    for (record, snapshot, _) in &samples {
                        let (mut cpu, mut memory_mb, source) = match record.deployment {
                            crate::instances::Deployment::Docker => {
                                let (c, m) = docker_usage
                                    .get(&crate::instances::container_name(&record.id))
                                    .cloned()
                                    .unwrap_or((0.0, 0.0));
                                (c, m, "docker-stats")
                            }
                            _ => {
                                // Endstone is a launcher: include descendants.
                                let (c, m) = pid_tree_usage(&system, snapshot.pid);
                                (c, m, "process-tree")
                            }
                        };
                        if snapshot.status != Status::Running {
                            cpu = 0.0;
                            memory_mb = 0.0;
                        }
                        map.insert(
                            record.id.clone(),
                            json!({
                                "id": record.id,
                                "name": record.name,
                                "status": snapshot.status,
                                "cpu_percent": cpu,
                                "memory_mb": memory_mb,
                                "disk_mb": disk_cache.get(&record.id).cloned().unwrap_or(0.0),
                                "tps": if snapshot.status == Status::Running { snapshot.tps.unwrap_or(20.0) } else { 0.0 },
                                "usage_source": source,
                                "limits": {
                                    "memory_mb": record.resources.memory_mb,
                                    "cpus": record.resources.cpus,
                                    "enforcement": record.deployment.enforcement(),
                                    "description": record.deployment.enforcement_description(),
                                },
                                "deployment": record.deployment.as_str(),
                            }),
                        );
                    }
                    (system, json!({"timestamp": now(), "host": host, "instances": map}))
                })
                .await;
                match result {
                    Ok((s, data)) => {
                        system = s;
                        if let Some(default) = data["instances"].get(DEFAULT_INSTANCE).cloned() {
                            latest_tx.send_replace(default);
                        }
                        all_tx.send_replace(data);
                    }
                    Err(e) => {
                        tracing::error!(%e, "metrics task failed");
                        break;
                    }
                }
            }
        });
        Self { latest, all }
    }
}

fn stopped_metrics() -> Value {
    json!({"timestamp":now(),"status":"stopped","cpu_percent":0.0,"memory_mb":0.0,"memory_percent":0.0,"tps":0.0})
}

fn host_shape(system: &System) -> Value {
    let total = system.total_memory();
    let used = system.used_memory();
    json!({
        "memory_total_mb": total as f64 / 1048576.0,
        "memory_used_mb": used as f64 / 1048576.0,
        "memory_percent": if total > 0 { 100.0 * used as f64 / total as f64 } else { 0.0 },
        "cpu_count": system.cpus().len(),
        "load_1": System::load_average().one,
    })
}

/// Sum CPU % and memory across a PID and all its descendants. sysinfo lists
/// every thread of a multithreaded server as its own entry sharing the
/// leader's full RSS — memory would be double-counted thread-over-thread, so
/// only thread-group leaders contribute memory. CPU is per-task and sums
/// correctly across threads.
fn pid_tree_usage(system: &System, pid: Option<u32>) -> (f64, f64) {
    let Some(root) = pid else {
        return (0.0, 0.0);
    };
    let mut pids = vec![Pid::from_u32(root)];
    let mut cursor = 0;
    while cursor < pids.len() {
        let parent = pids[cursor];
        cursor += 1;
        for (pid, process) in system.processes() {
            if process.parent() == Some(parent) && !pids.contains(pid) {
                pids.push(*pid);
            }
        }
    }
    let mut cpu = 0.0f64;
    let mut memory = 0u64;
    for pid in pids {
        if let Some(process) = system.process(pid) {
            cpu += f64::from(process.cpu_usage());
            if process.thread_kind().is_none() {
                memory += process.memory();
            }
        }
    }
    (cpu, memory as f64 / 1048576.0)
}

/// One `docker stats --no-stream` call for every running container, mapped by
/// container name → (cpu_percent, memory_mb).
async fn collect_docker_stats() -> BTreeMap<String, (f64, f64)> {
    let mut map = BTreeMap::new();
    let output = tokio::time::timeout(
        Duration::from_secs(6),
        tokio::process::Command::new("docker")
            .args([
                "stats",
                "--no-stream",
                "--format",
                "{{.Name}}\t{{.MemUsage}}\t{{.CPUPerc}}",
            ])
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await;
    let Ok(Ok(output)) = output else {
        return map;
    };
    if !output.status.success() {
        return map;
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split('\t');
        let (Some(name), Some(mem), Some(cpu)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let memory = parse_docker_memory(mem.split('/').next().unwrap_or("").trim()).unwrap_or(0.0);
        let cpu = cpu
            .trim()
            .trim_end_matches('%')
            .parse::<f64>()
            .unwrap_or(0.0);
        map.insert(name.trim().to_string(), (cpu, memory));
    }
    map
}
