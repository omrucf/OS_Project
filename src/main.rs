use procfs::process::all_processes;
use procfs::ProcResult;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> ProcResult<()> {
    // Display system load average
    let load_avg = procfs::LoadAverage::new()?;
    println!(
        "Load Average: 1 min: {}, 5 min: {}, 15 min: {}",
        load_avg.one, load_avg.five, load_avg.fifteen
    );

    // Display memory information
    let mem_info = procfs::Meminfo::new()?;
    println!(
        "Memory Usage: {} kB total, {} kB free, {} kB available",
        mem_info.mem_total,
        mem_info.mem_free,
        mem_info.mem_available.unwrap_or(0)
    );

    // Calculate system uptime
    let kernel_stats = procfs::KernelStats::new()?;
    let btime = kernel_stats.btime; // Directly use the value as it is already a u64
    let uptime = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() - btime,
        Err(_) => {
            eprintln!("Failed to calculate system time.");
            0
        }
    };
    println!("System Uptime: {} seconds", uptime);

    // Display running processes with their CPU usage
    println!("{:<6} {:<20} {:<10} {:<10}", "PID", "Command", "CPU%", "Memory%");
    for process in all_processes()? {
        match process {
            Ok(proc) => {
                if let Ok(stat) = proc.stat() {
                    let cpu_usage = calculate_cpu_usage(&stat, uptime);
                    let mem_usage = (stat.rss as f64 / mem_info.mem_total as f64) * 100.0;
                    println!(
                        "{:<6} {:<20} {:<10.2} {:<10.2}",
                        stat.pid, stat.comm, cpu_usage, mem_usage
                    );
                }
            }
            Err(err) => {
                eprintln!("Failed to get process info: {}", err);
            }
        }
    }

    Ok(())
}

fn calculate_cpu_usage(stat: &procfs::process::Stat, uptime: u64) -> f64 {
    // Calculate CPU usage percentage
    let total_time = stat.utime + stat.stime;
    let hertz = procfs::ticks_per_second().unwrap_or(100) as f64; // Default value for hertz
    let seconds = uptime as f64 - (stat.starttime as f64 / hertz);
    if seconds > 0.0 {
        (total_time as f64 / hertz) / seconds * 100.0
    } else {
        0.0
    }
}
