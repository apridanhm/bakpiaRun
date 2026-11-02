use serde::Serialize;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::pool_manager::PoolManager;

/// Memory telemetry of an individual Linux process read from `/proc/[pid]/status`
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProcMemoryStatus {
    pub pid: u32,
    /// Resident Set Size (actual physical RAM used by process in bytes)
    pub vm_rss_bytes: u64,
    /// High Water Mark (peak physical RAM used by process in bytes)
    pub vm_hwm_bytes: u64,
    /// Virtual Memory Size in bytes
    pub vm_size_bytes: u64,
}

/// Container / Host aggregate memory telemetry read from `cgroups v2` (with fallback to cgroup v1 / `/proc/meminfo`)
#[derive(Debug, Clone, Default, Serialize)]
pub struct CgroupMemoryStatus {
    /// Source of telemetry: "cgroup_v2", "cgroup_v1", or "proc_meminfo"
    pub source: String,
    /// Current aggregate memory usage in bytes
    pub current_bytes: u64,
    /// Maximum allowed memory limit in bytes (0 if unlimited)
    pub max_bytes: u64,
    /// Percentage of memory limit currently used (0.0 - 100.0)
    pub usage_percent: f64,
}

/// Reads `/proc/[pid]/status` on Linux to extract `VmRSS`, `VmHWM`, and `VmSize`.
pub fn read_process_proc_status(pid: u32) -> Option<ProcMemoryStatus> {
    let path = format!("/proc/{}/status", pid);
    let content = fs::read_to_string(&path).ok()?;

    let mut status = ProcMemoryStatus {
        pid,
        ..Default::default()
    };

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("VmRSS:") {
            status.vm_rss_bytes = parse_kb_line(val);
        } else if let Some(val) = line.strip_prefix("VmHWM:") {
            status.vm_hwm_bytes = parse_kb_line(val);
        } else if let Some(val) = line.strip_prefix("VmSize:") {
            status.vm_size_bytes = parse_kb_line(val);
        }
    }

    Some(status)
}

fn parse_kb_line(raw: &str) -> u64 {
    // Format in /proc/[pid]/status is e.g. "   18432 kB"
    let trimmed = raw.trim();
    let num_str = trimmed.split_whitespace().next().unwrap_or("0");
    num_str.parse::<u64>().unwrap_or(0) * 1024
}

/// Reads aggregate container memory from `cgroups v2` (`memory.current` and `memory.max`),
/// falling back to `cgroups v1` or `/proc/meminfo` if `memory.max` is `"max"` or unavailable.
pub fn read_cgroup_v2_memory(current_path: &str, max_path: &str) -> CgroupMemoryStatus {
    // 1. Try cgroups v2 first
    if let Ok(curr_raw) = fs::read_to_string(current_path) {
        if let Ok(current_bytes) = curr_raw.trim().parse::<u64>() {
            let max_raw = fs::read_to_string(max_path).unwrap_or_else(|_| "max".to_string());
            let max_trimmed = max_raw.trim();

            let max_bytes = if max_trimmed == "max" {
                // Container has no hard cgroup v2 limit ("max"), use total system RAM from /proc/meminfo as ceiling
                read_host_memtotal_bytes().unwrap_or(0)
            } else {
                max_trimmed.parse::<u64>().unwrap_or(0)
            };

            let usage_percent = if max_bytes > 0 {
                (current_bytes as f64 / max_bytes as f64) * 100.0
            } else {
                0.0
            };

            return CgroupMemoryStatus {
                source: "cgroup_v2".to_string(),
                current_bytes,
                max_bytes,
                usage_percent,
            };
        }
    }

    // 2. Fallback: cgroups v1
    let v1_curr = "/sys/fs/cgroup/memory/memory.usage_in_bytes";
    let v1_max = "/sys/fs/cgroup/memory/memory.limit_in_bytes";
    if let Ok(curr_raw) = fs::read_to_string(v1_curr) {
        if let Ok(current_bytes) = curr_raw.trim().parse::<u64>() {
            let max_bytes = fs::read_to_string(v1_max)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .filter(|&m| m < (1u64 << 60))
                .or_else(read_host_memtotal_bytes)
                .unwrap_or(0);

            let usage_percent = if max_bytes > 0 {
                (current_bytes as f64 / max_bytes as f64) * 100.0
            } else {
                0.0
            };

            return CgroupMemoryStatus {
                source: "cgroup_v1".to_string(),
                current_bytes,
                max_bytes,
                usage_percent,
            };
        }
    }

    // 3. Fallback: /proc/meminfo
    if let Some((used_bytes, total_bytes)) = read_host_meminfo_used_and_total() {
        let usage_percent = if total_bytes > 0 {
            (used_bytes as f64 / total_bytes as f64) * 100.0
        } else {
            0.0
        };
        return CgroupMemoryStatus {
            source: "proc_meminfo".to_string(),
            current_bytes: used_bytes,
            max_bytes: total_bytes,
            usage_percent,
        };
    }

    CgroupMemoryStatus {
        source: "unavailable".to_string(),
        current_bytes: 0,
        max_bytes: 0,
        usage_percent: 0.0,
    }
}

fn read_host_memtotal_bytes() -> Option<u64> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("MemTotal:") {
            return Some(parse_kb_line(val));
        }
    }
    None
}

fn read_host_meminfo_used_and_total() -> Option<(u64, u64)> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    let mut total_bytes = 0u64;
    let mut available_bytes = 0u64;

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("MemTotal:") {
            total_bytes = parse_kb_line(val);
        } else if let Some(val) = line.strip_prefix("MemAvailable:") {
            available_bytes = parse_kb_line(val);
        }
    }

    if total_bytes > 0 {
        let used = total_bytes.saturating_sub(available_bytes);
        Some((used, total_bytes))
    } else {
        None
    }
}

/// Proactive Anti-OOM Engine background task.
/// Periodically inspects `/proc/[pid]/status` for every worker across all pools
/// and `cgroups v2` aggregate container memory, performing graceful worker recycling
/// when a worker or container approaches its memory threshold.
pub struct AntiOomEngine;

impl AntiOomEngine {
    pub fn start_monitor(
        config_arc: Arc<Mutex<Config>>,
        pool_manager_arc: Arc<Mutex<PoolManager>>,
    ) {
        tokio::spawn(async move {
            println!("[Anti-OOM Engine] Proactive background monitor started (/proc/[pid]/status + cgroups v2)");

            loop {
                let (enabled, interval_ms, cgroup_curr_path, cgroup_max_path, cgroup_threshold_pct) = {
                    let cfg = config_arc.lock().await;
                    (
                        cfg.anti_oom.enabled,
                        cfg.anti_oom.check_interval_ms.max(200),
                        cfg.anti_oom.cgroup_memory_current_path.clone(),
                        cfg.anti_oom.cgroup_memory_max_path.clone(),
                        cfg.anti_oom.cgroup_threshold_percent,
                    )
                };

                tokio::time::sleep(Duration::from_millis(interval_ms)).await;

                if !enabled {
                    continue;
                }

                // 1. Check aggregate cgroup v2 memory pressure
                let cgroup_mem = read_cgroup_v2_memory(&cgroup_curr_path, &cgroup_max_path);
                let cgroup_pressure = cgroup_mem.max_bytes > 0
                    && cgroup_mem.usage_percent >= (cgroup_threshold_pct as f64);

                if cgroup_pressure {
                    println!(
                        "[Anti-OOM Engine] ⚠️ Container cgroups v2 memory pressure detected: {:.1}% ({} MB / {} MB, threshold: {}%)",
                        cgroup_mem.usage_percent,
                        cgroup_mem.current_bytes / 1024 / 1024,
                        cgroup_mem.max_bytes / 1024 / 1024,
                        cgroup_threshold_pct
                    );
                }

                // 2. Inspect each pool and refresh /proc/[pid]/status VmRSS for all workers
                let pools_snapshot = {
                    let pm = pool_manager_arc.lock().await;
                    pm.pools
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<Vec<_>>()
                };

                let mut heaviest_candidate: Option<(String, Arc<Mutex<crate::worker::Worker>>, Config, u64)> = None;

                for (pool_name, pool_arc) in &pools_snapshot {
                    let (workers_list, pool_config) = {
                        let pool = pool_arc.lock().await;
                        (pool.workers.clone(), pool.config.clone())
                    };

                    for worker_arc in workers_list {
                        let mut worker = worker_arc.lock().await;
                        worker.refresh_proc_memory();

                        // Check if individual worker exceeded its proactive memory or request threshold
                        if let Some(reason) = worker.should_restart(&pool_config) {
                            println!(
                                "[Anti-OOM Engine] Pool '{}' {} -> initiating graceful worker recycling",
                                pool_name, reason
                            );
                            if let Err(e) = worker.graceful_recycle(&pool_config).await {
                                eprintln!(
                                    "[Anti-OOM Engine] Failed to recycle worker #{} in pool '{}': {}",
                                    worker.index, pool_name, e
                                );
                            }
                            continue;
                        }

                        // Track heaviest idle/active worker in case container cgroup memory pressure requires proactive eviction
                        if cgroup_pressure && worker.rss_bytes > 0 {
                            match &heaviest_candidate {
                                Some((_, _, _, max_rss)) if worker.rss_bytes <= *max_rss => {}
                                _ => {
                                    heaviest_candidate = Some((
                                        pool_name.clone(),
                                        worker_arc.clone(),
                                        pool_config.clone(),
                                        worker.rss_bytes,
                                    ));
                                }
                            }
                        }
                    }
                }

                // 3. If container cgroup memory is above threshold and no individual worker was recycled yet,
                // proactively recycle the worker with the highest VmRSS across pools.
                if cgroup_pressure {
                    if let Some((target_pool, worker_arc, pool_config, rss_bytes)) = heaviest_candidate {
                        let mut worker = worker_arc.lock().await;
                        println!(
                            "[Anti-OOM Engine] Recycling heaviest worker #{} in pool '{}' (VmRSS: {} MB) due to cgroup memory pressure",
                            worker.index,
                            target_pool,
                            rss_bytes / 1024 / 1024
                        );
                        let _ = worker.graceful_recycle(&pool_config).await;
                    }
                }
            }
        });
    }
}

