use procfs::process::all_processes;
use procfs::{ProcResult, process::Stat};
use users::{get_user_by_uid};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use chrono::Local;
use std::io::{self, Write};
use libc::{kill, SIGKILL};

fn main() -> ProcResult<()> {
    let kernel_stats = procfs::KernelStats::new()?;
    let btime = kernel_stats.btime;

    loop {
        // Clear screen for live updates
        clear_screen();

        // Display system stats
        display_system_stats(uptime(&btime))?;

        // Display process information
        display_processes(uptime(&btime))?;

        // Prompt the user for actions (e.g., kill a process)
        prompt_user()?;

        // Refresh every 2 seconds
        // sleep(Duration::from_secs(2));
    }
}

fn display_system_stats(uptime: u64) -> ProcResult<()> {
    let load_avg = procfs::LoadAverage::new()?;
    let mem_info = procfs::Meminfo::new()?;
    let total_mem_mb = mem_info.mem_total as f64 / 1024.0;
    let free_mem_mb = mem_info.mem_free as f64 / 1024.0;
    let buffers_mb = mem_info.buffers as f64 / 1024.0;
    let cached_mb = mem_info.cached as f64 / 1024.0;
    let used_mem_mb = total_mem_mb - free_mem_mb - buffers_mb - cached_mb;

    println!(
        "top - {}  up {} seconds,  load average: {:.2}, {:.2}, {:.2}",
        Local::now().format("%H:%M:%S"),
        uptime,
        load_avg.one,
        load_avg.five,
        load_avg.fifteen
    );

    println!(
        "MiB Mem : {:>8.1} total, {:>8.1} used, {:>8.1} free, {:>8.1} buff/cache",
        total_mem_mb, used_mem_mb, free_mem_mb, buffers_mb + cached_mb
    );

    Ok(())
}

struct Process {
    pid: i32,
    user: String,
    priority: i64,
    nice: i64,
    virt: f64,
    res: f64,
    shr: f64,
    cpu_usage: f64,
    mem_usage: f64,
    time_plus: String,
    command: String,
}

fn display_processes(uptime: u64) -> ProcResult<()> {
    println!(
        "{:<8} {:<15} {:<6} {:<6} {:<12} {:<12} {:<12} {:<8} {:<8} {:<12} {:<25}",
        "PID", "USER", "PR", "NI", "VIRT", "RES", "SHR", "%CPU", "%MEM", "TIME+", "COMMAND"
    );

    let mut processes: Vec<Process> = vec![];

    for process in all_processes()? {
        if let Ok(proc) = process {
            if let Ok(stat) = proc.stat() {
                if let Ok(status) = proc.status() {
                    let cpu_usage = calculate_cpu_usage(&stat, uptime);
                    let mem_usage = calculate_memory_usage(&stat);
                    let time_plus = format_time(stat.utime + stat.stime);
                    let user = get_user(status.ruid);
                    let priority = stat.priority;
                    let nice = stat.nice;
                    let virt = stat.vsize as f64 / 1024.0; // Virtual memory in KB
                    let res = calculate_res_memory(&stat);
                    let shr = calculate_shr_memory(&stat);

                    processes.push(Process {
                        pid: stat.pid,
                        user,
                        priority,
                        nice,
                        virt,
                        res,
                        shr,
                        cpu_usage,
                        mem_usage,
                        time_plus,
                        command: stat.comm,
                    });
                }
            }
        }
    }

    // Sort by CPU usage in descending order
    processes.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap());
    let top_processes = processes.into_iter().take(30);

    for proc in top_processes {
        println!(
            "{:<8} {:<15} {:<6} {:<6} {:<12.1} {:<12.1} {:<12.1} {:<8.2} {:<8.2} {:<12} {:<25}",
            proc.pid, proc.user, proc.priority, proc.nice, proc.virt, proc.res, proc.shr, proc.cpu_usage, proc.mem_usage, proc.time_plus, proc.command
        );
    }

    Ok(())
}

fn prompt_user() -> ProcResult<()> {
    print!("Enter PID to kill (or press Enter to skip): ");
    io::stdout().flush().unwrap(); // Ensure the prompt is displayed immediately

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    if let Ok(pid) = input.trim().parse::<i32>() {
        // If a valid PID is entered, attempt to kill the process
        if kill_process(pid) {
            println!("Process with PID {} has been killed.", pid);
        } else {
            println!("Failed to kill process with PID {}. Check permissions.", pid);
        }
    }

    Ok(())
}

fn kill_process(pid: i32) -> bool {
    // Use libc's kill function to send the SIGKILL signal
    unsafe { kill(pid, SIGKILL) == 0 }
}

fn uptime(btime: &u64) -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() - btime,
        Err(_) => 0,
    }
}

fn calculate_cpu_usage(stat: &Stat, uptime: u64) -> f64 {
    let total_time = stat.utime + stat.stime + (stat.cutime + stat.cstime) as u64;
    let hertz = procfs::ticks_per_second().unwrap_or(100) as f64;
    let elapsed_time = uptime as f64 - (stat.starttime as f64 / hertz);
    if elapsed_time > 0.0 {
        ((total_time as f64 / hertz) / elapsed_time) * 100.0
    } else {
        0.0
    }
}

fn calculate_memory_usage(stat: &Stat) -> f64 {
    let page_size_kb = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as f64 } / 1024.0;
    (stat.rss as f64 * page_size_kb) / 1024.0
}

fn calculate_res_memory(stat: &Stat) -> f64 {
    let page_size_kb = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as f64 } / 1024.0;
    stat.rss as f64 * page_size_kb
}

fn calculate_shr_memory(stat: &Stat) -> f64 {
    calculate_res_memory(stat) / 2.0
}

fn format_time(clock_ticks: u64) -> String {
    let seconds = clock_ticks as f64 / procfs::ticks_per_second().unwrap_or(100) as f64;
    let minutes = (seconds as u64 / 60) % 60;
    let hours = seconds as u64 / 3600;
    let seconds = seconds as u64 % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

fn get_user(uid: u32) -> String {
    get_user_by_uid(uid)
        .map(|user| user.name().to_string_lossy().to_string())
        .unwrap_or_else(|| "N/A".to_string())
}

fn clear_screen() {
    print!("\x1B[2J\x1B[H");
}
