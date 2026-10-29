//! Small shared utilities for the instance subsystem: instance id/slug
//! rules, Docker container naming, and pure host-capacity warning math.

/// Valid instance ids: lowercase slugs so they are safe as directory names,
/// unit names, and container names.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// Derive an instance id from a display name ("Dad's Server!" → "dads-server").
pub fn slug(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug.trim_matches(['-']).chars().take(48).collect()
}

/// Docker container name for an instance id.
pub fn container_name(id: &str) -> String {
    format!("omnibedrock-{id}")
}

/// Judge aggregate instance allocations against host capacity minus the
/// configured reserve. Pure so the exact warning logic is unit-testable.
/// Returns (memory_warning, cpu_warning); each is None when healthy.
pub fn overcommit_status(
    host_memory_mb: f64,
    reserve_mb: u64,
    allocated_memory_mb: u64,
    cpu_count: usize,
    allocated_cpus: f64,
) -> (Option<String>, Option<String>) {
    let safe_memory = (host_memory_mb - reserve_mb as f64).max(0.0);
    let memory = if allocated_memory_mb as f64 > safe_memory {
        Some(format!(
            "allocated {allocated_memory_mb} MiB exceeds the safe budget of {:.0} MiB              (host {:.0} MiB − {reserve_mb} MiB reserve); instances may contend or hit OOM",
            safe_memory, host_memory_mb
        ))
    } else {
        None
    };
    let cpu = if allocated_cpus > cpu_count as f64 + 1e-9 {
        Some(format!(
            "allocated {allocated_cpus:.2} CPU across instances exceeds the host's              {cpu_count} cores; CPU limits are quotas and will throttle under load"
        ))
    } else {
        None
    };
    (memory, cpu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_id_rules() {
        assert_eq!(slug("Dad's Server!"), "dad-s-server");
        assert!(valid_id("dad-s-server"));
        assert!(!valid_id("Dad"));
        assert!(!valid_id(""));
        assert!(!valid_id("-x"));
        assert!(!valid_id("x-"));
    }

    #[test]
    fn overcommit_warnings_respect_reserve() {
        // 8192 total − 1024 reserve = 7168 safe; 7000 fits, 8000 warns.
        let (none, cpu_none) = overcommit_status(8192.0, 1024, 7000, 8, 7.5);
        assert!(none.is_none() && cpu_none.is_none());
        let (warn, _) = overcommit_status(8192.0, 1024, 8000, 8, 7.5);
        assert!(warn.unwrap().contains("exceeds the safe budget"));
        // Reserve larger than the host: everything warns, never a negative budget.
        let (warn, _) = overcommit_status(512.0, 4096, 128, 4, 0.5);
        assert!(warn.unwrap().contains("exceeds the safe budget"));
        // CPU overcommit is independent of memory health.
        let (_, cpu) = overcommit_status(65536.0, 1024, 1024, 4, 6.0);
        assert!(cpu.unwrap().contains("exceeds the host's"));
    }
}
