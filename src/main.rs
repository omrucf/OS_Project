use procfs::process::all_processes;
use procfs::{process::Stat};
use users::{get_user_by_uid};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use chrono::Local;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
    Terminal,
};
use libc::{kill, SIGKILL, SIGSTOP, SIGCONT};

#[derive(PartialEq, Eq)]
enum SortCriteria {
    CPU,
    Memory,
    PID,
}

struct Process {
    pid: i32,
    ppid: i32,
    user: String,
    threads: i64,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Clear the terminal before starting
    print!("\x1B[2J\x1B[H");

    let mut terminal = setup_terminal()?;
    let kernel_stats = procfs::KernelStats::new()?;
    let btime = kernel_stats.btime;

    let mut scroll_offset = 0;
    let mut selected_index = 0; // Tracks the currently selected process
    let mut sort_criteria = SortCriteria::CPU; // Default sorting by CPU usage

    loop {
        // Collect system and process data
        let uptime = uptime(&btime);
        let load_avg = procfs::LoadAverage::new()?;
        let mem_info = procfs::Meminfo::new()?;
        let total_mem_mb = mem_info.mem_total as f64 / 1024.0;
        let free_mem_mb = mem_info.mem_free as f64 / 1024.0;
        let buffers_mb = mem_info.buffers as f64 / 1024.0;
        let cached_mb = mem_info.cached as f64 / 1024.0;
        let used_mem_mb = total_mem_mb - free_mem_mb - buffers_mb - cached_mb;

        let system_stats = format!(
            "top - {}  up {} seconds,  load average: {:.2}, {:.2}, {:.2}\n\
            MiB Mem : {:>8.1} total, {:>8.1} used, {:>8.1} free, {:>8.1} buff/cache",
            Local::now().format("%H:%M:%S"),
            uptime,
            load_avg.one,
            load_avg.five,
            load_avg.fifteen,
            total_mem_mb, used_mem_mb, free_mem_mb, buffers_mb + cached_mb
        );

        let mut processes = vec![];

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
                            ppid: stat.ppid,
                            user,
                            threads: stat.num_threads,
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

        // Sort processes based on the current sorting criteria
        match sort_criteria {
            SortCriteria::CPU => {
                processes.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap());
            }
            SortCriteria::Memory => {
                processes.sort_by(|a, b| b.mem_usage.partial_cmp(&a.mem_usage).unwrap());
            }
            SortCriteria::PID => {
                processes.sort_by(|a, b| a.pid.cmp(&b.pid));
            }
        }

        // Draw the TUI
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Percentage(20), // System stats
                        Constraint::Percentage(60), // Process list (shortened)
                        Constraint::Percentage(20), // Help section
                    ]
                    .as_ref(),
                )
                .split(f.area());

            draw_system_stats(f, chunks[0], &system_stats);
            draw_process_list(f, chunks[1], &processes, scroll_offset, selected_index);
            draw_help_section(f, chunks[2], &sort_criteria);
        })?;

        // Handle user input
        if event::poll(Duration::from_secs(1))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        reset_terminal(terminal)?;
                        break;
                    }
                    KeyCode::Char('k') => {
                        if let Some(proc) = processes.get(selected_index) {
                            if unsafe { kill(proc.pid, SIGKILL) } == 0 {
                                // println!("Killed process with PID {}", proc.pid);
                            } else {
                                println!("Failed to kill process with PID {}. Check permissions.", proc.pid);
                            }
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(proc) = processes.get(selected_index) {
                            if unsafe { kill(proc.pid, SIGSTOP) } == 0 {
                                // println!("Suspended process with PID {}", proc.pid);
                            } else {
                                println!("Failed to suspend process with PID {}. Check permissions.", proc.pid);
                            }
                        }
                    }
                    KeyCode::Char('w') => {
                        if let Some(proc) = processes.get(selected_index) {
                            if unsafe { kill(proc.pid, SIGCONT) } == 0 {
                                // println!("Resumed process with PID {}", proc.pid);
                            } else {
                                println!("Failed to resume process with PID {}. Check permissions.", proc.pid);
                            }
                        }
                    }
                    KeyCode::Char('c') => {
                        sort_criteria = SortCriteria::CPU;
                    }
                    KeyCode::Char('m') => {
                        sort_criteria = SortCriteria::Memory;
                    }
                    KeyCode::Char('p') => {
                        sort_criteria = SortCriteria::PID;
                    }
                    KeyCode::Down => {
                        if selected_index < processes.len() - 1 {
                            selected_index += 1;
                            if selected_index >= scroll_offset + 10 {
                                scroll_offset += 1;
                            }
                        }
                    }
                    KeyCode::Up => {
                        if selected_index > 0 {
                            selected_index -= 1;
                            if selected_index < scroll_offset {
                                scroll_offset -= 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn draw_process_list(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    processes: &[Process],
    scroll_offset: usize,
    selected_index: usize,
) {
    let rows: Vec<Row> = processes
        .iter()
        .skip(scroll_offset)
        .take(10)
        .enumerate()
        .map(|(i, p)| {
            let style = if scroll_offset + i == selected_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                p.pid.to_string(),
                p.ppid.to_string(),
                p.user.clone(),
                p.threads.to_string(),
                format!("{:.1}", p.cpu_usage),
                format!("{:.1} MB", p.mem_usage),
                p.time_plus.clone(),
                p.command.clone(),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),   // PID
            Constraint::Length(6),   // PPID
            Constraint::Length(10),  // User
            Constraint::Length(8),   // Threads
            Constraint::Length(8),   // CPU
            Constraint::Length(10),  // Memory
            Constraint::Length(12),  // Time+
            Constraint::Min(20),     // Command
        ],
    )
    .header(Row::new(vec!["PID", "PPID", "USER", "THR", "%CPU", "MEM", "TIME+", "COMMAND"]))
    .block(Block::default().title("Processes").borders(Borders::ALL));

    f.render_widget(table, area);
}

fn draw_help_section(f: &mut ratatui::Frame, area: ratatui::layout::Rect, sort_criteria: &SortCriteria) {
    let sort_label = match sort_criteria {
        SortCriteria::CPU => "Sorting by: CPU",
        SortCriteria::Memory => "Sorting by: Memory",
        SortCriteria::PID => "Sorting by: PID",
    };
    let help_text = format!(
        "{}\nKeys: q: Quit  k: Kill  s: Suspend  w: Resume  c: CPU  m: Memory  p: PID  ↑/↓: Navigate",
        sort_label
    );
    let block = Block::default().title("Help").borders(Borders::ALL);
    let paragraph = Paragraph::new(help_text).block(block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(paragraph, area);
}

fn draw_system_stats(f: &mut ratatui::Frame, area: ratatui::layout::Rect, stats: &String) {
    let block = Block::default().title("System Stats").borders(Borders::ALL);
    let paragraph = Paragraph::new(stats.clone()).block(block);
    f.render_widget(paragraph, area);
}
// Helper functions like uptime, calculate_cpu_usage, etc., remain unchanged

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>, Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    let stdout = std::io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn reset_terminal<B: Backend>(mut terminal: Terminal<B>) -> Result<(), Box<dyn std::error::Error>> {
    // Clear the terminal before exiting
    print!("\x1B[2J\x1B[H");
    crossterm::terminal::disable_raw_mode()?;
    terminal.show_cursor()?;
    Ok(())
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
