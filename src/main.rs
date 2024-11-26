use procfs::process::all_processes;
use procfs::process::Stat;
use users::get_user_by_uid;
use std::collections::HashMap;
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
use std::process::Command;




#[derive(PartialEq, Eq, Clone)]
enum SortCriteria {
    CPU,
    Memory,
    PID,
    PR,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum ViewState {
    Processes,
    CrashTracking,
    ProcessTree,
}

#[derive(Clone)]
struct Process {
    pid: i32,
    ppid: i32,
    user: String,
    state: char,
    threads: i64,
    priority: i64,
    cpu_usage: f64,
    mem_usage: f64,
    time_plus: String,
    command: String,
    children: HashMap<i32, Process>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    print!("\x1B[2J\x1B[H");

    let mut terminal = setup_terminal()?;
    let kernel_stats = procfs::KernelStats::new()?;
    let btime = kernel_stats.btime;

    let mut scroll_offset = 0;
    let mut selected_index = 0;
    let mut sort_criteria = SortCriteria::CPU;
    let mut view_state = ViewState::Processes;
    let mut tree_view_pid = None;

    

    let view_states = vec![ViewState::Processes, ViewState::CrashTracking, ViewState::ProcessTree];
    let mut view_i = 0;

    loop {
        let uptime = uptime(&btime);
        let load_avg = procfs::LoadAverage::new()?;
        let mem_info = procfs::Meminfo::new()?;
        let total_mem_mb = mem_info.mem_total as f64 / 1024.0;
        let free_mem_mb = mem_info.mem_free as f64 / 1024.0;
        let buffers_mb = mem_info.buffers as f64 / 1024.0;
        let cached_mb = mem_info.cached as f64 / 1024.0;
        let used_mem_mb = total_mem_mb - free_mem_mb - buffers_mb - cached_mb;

        let crash_history = get_crash_logs();

        let system_stats = format!(
            "this - {}  up {} seconds,  load average: {:.2}, {:.2}, {:.2}\n\
            MiB Mem : {:>8.1} total, {:>8.1} used, {:>8.1} free, {:>8.1} buff/cache",
            Local::now().format("%H:%M:%S"),
            uptime,
            load_avg.one,
            load_avg.five,
            load_avg.fifteen,
            total_mem_mb, used_mem_mb, free_mem_mb, buffers_mb + cached_mb
        );

        let mut process_map: HashMap<i32, Process> = HashMap::new();
        for process in all_processes()? {
            if let Ok(proc) = process {
                if let Ok(stat) = proc.stat() {
                    if let Ok(status) = proc.status() {
                        let cpu_usage = calculate_cpu_usage(&stat, uptime);
                        let mem_usage = calculate_memory_usage(&stat);
                        let time_plus = format_time(stat.utime + stat.stime);
                        let user = get_user(status.ruid);
                        let priority = stat.priority;

                        let proc = Process {
                            pid: stat.pid,
                            ppid: stat.ppid,
                            user,
                            state: stat.state,
                            threads: stat.num_threads,
                            priority,
                            cpu_usage,
                            mem_usage,
                            time_plus,
                            command: stat.comm,
                            children: HashMap::new(),
                        };

                        process_map.insert(proc.pid, proc);
                    }
                }
            }
        }

        let mut children_map: HashMap<i32, Vec<Process>> = HashMap::new();
        for process in process_map.values() {
            children_map.entry(process.ppid).or_default().push(process.clone());
        }

        for process in process_map.values_mut() {
            if let Some(children) = children_map.remove(&process.pid) {
                for child in children {
                    process.children.insert(child.pid, child);
                }
            }
        }

        // Sort processes if in the Processes view
        let mut processes: Vec<&Process> = process_map.values().collect();
        if view_state == ViewState::Processes {
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
                SortCriteria::PR => {
                    processes.sort_by(|a, b| b.priority.cmp(&a.priority));
                }
            }
        }

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Percentage(20),
                        Constraint::Percentage(60),
                        Constraint::Percentage(20),
                    ]
                    .as_ref(),
                )
                .split(f.area());

            draw_system_stats(f, chunks[0], &system_stats);

            match view_state {
                ViewState::Processes => {
                    let processes_for_display: Vec<Process> = processes.iter().cloned().cloned().collect();
                    draw_process_list(
                        f,
                        chunks[1],
                        &processes_for_display,
                        scroll_offset,
                        selected_index,
                    );
                }
                ViewState::CrashTracking => {
                    draw_crash_tracking(f, chunks[1], &crash_history);
                }
                ViewState::ProcessTree => {
                    if let Some(pid) = tree_view_pid {
                        draw_process_tree(f, chunks[1], pid, &process_map);
                    } else {
                        draw_empty_tree_view(f, chunks[1]);
                    }
                }
            }

            draw_help_section(f, chunks[2], &sort_criteria, &view_state);
        })?;

        if event::poll(Duration::from_secs(1))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        reset_terminal(terminal)?;
                        break;
                    }
                    KeyCode::Char('k') => {
                        if view_state == ViewState::Processes {
                            if let Some(proc) = processes.get(selected_index) {
                                if unsafe { kill(proc.pid, SIGKILL) } == 0 {
                                    // println!("Killed process with PID {}", proc.pid);
                                } else {
                                    println!(
                                        "Failed to kill process with PID {}. Check permissions.",
                                        proc.pid
                                    );
                                }
                            }
                        }
                    }
                    KeyCode::Char('t') => {
                        if view_state == ViewState::Processes {
                            if let Some(proc) = processes.get(selected_index) {
                                tree_view_pid = Some(proc.pid);
                                view_state = ViewState::ProcessTree;
                            }
                        }
                    }

                    KeyCode::Char('s') => {
                        if view_state == ViewState::Processes {
                            if let Some(proc) = processes.get(selected_index) {
                                if unsafe { kill(proc.pid, SIGSTOP) } == 0 {
                                    // println!("Suspended process with PID {}", proc.pid);
                                } else {
                                    println!(
                                        "Failed to suspend process with PID {}. Check permissions.",
                                        proc.pid
                                    );
                                }
                            }
                        }
                    }
                    KeyCode::Char('w') => {
                        if view_state == ViewState::Processes {
                            if let Some(proc) = processes.get(selected_index) {
                                if unsafe { kill(proc.pid, SIGCONT) } == 0 {
                                    // println!("Resumed process with PID {}", proc.pid);
                                } else {
                                    println!(
                                        "Failed to resume process with PID {}. Check permissions.",
                                        proc.pid
                                    );
                                }
                            }
                        }
                    }
                    KeyCode::Left => {
                        view_i = (view_i + 2) % 3;
                        view_state = view_states[view_i].clone();
                    }
                    KeyCode::Right => {
                        view_i = (view_i + 1) % 3;
                        view_state = view_states[view_i].clone();
                    }
                    KeyCode::Down => {
                        if view_state == ViewState::Processes {
                            if selected_index < processes.len() - 1 {
                                selected_index += 1;
                                if selected_index >= scroll_offset + 20 {
                                    scroll_offset += 1;
                                }
                            }
                        }
                    }
                    KeyCode::Up => {
                        if view_state == ViewState::Processes {
                            if selected_index > 0 {
                                selected_index -= 1;
                                if selected_index < scroll_offset {
                                    scroll_offset -= 1;
                                }
                            }
                        }
                    }
                    KeyCode::Char('c') => {
                        if view_state == ViewState::Processes {
                            sort_criteria = SortCriteria::CPU;
                        }
                    }
                    KeyCode::Char('m') => {
                        if view_state == ViewState::Processes {
                            sort_criteria = SortCriteria::Memory;
                        }
                    }
                    KeyCode::Char('p') => {
                        if view_state == ViewState::Processes {
                            sort_criteria = SortCriteria::PID;
                        }
                    }
                    KeyCode::Char('r') => {
                        if view_state == ViewState::Processes {
                            sort_criteria = SortCriteria::PR;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}


fn get_crash_logs() -> Vec<String> {
    // Run `dmesg` and capture the output
    let output = Command::new("dmesg")
        .arg("--ctime") // Include human-readable timestamps
        .output()
        .expect("Failed to execute dmesg");

    let logs = String::from_utf8_lossy(&output.stdout);
    logs.lines()
        .filter(|line| line.contains("segfault") || line.contains("oom"))
        .map(|line| line.to_string())
        .collect()
}

fn draw_crash_tracking(f: &mut ratatui::Frame, area: ratatui::layout::Rect, crash_history: &[String]) {
    let block = Block::default().title("Crash Tracking").borders(Borders::ALL);
    let content = crash_history.join("\n");
    let paragraph = Paragraph::new(content).block(block).style(
        Style::default()
    );
    f.render_widget(paragraph, area);
}


fn draw_process_tree(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    pid: i32,
    process_map: &HashMap<i32, Process>,
) {
    let mut content = String::new();

    // Find the selected process
    if let Some(proc) = process_map.get(&pid) {
        content.push_str(&format!("Process: {} ({})\n", proc.pid, proc.command));

        content.push_str("\nParents:\n");
        let mut current_pid = Some(proc.ppid);
        let mut level = 0;
        while let Some(ppid) = current_pid {
            if let Some(parent) = process_map.get(&ppid) {
                content.push_str(&format!(
                    "{}- {} ({})\n",
                    "  ".repeat(level),
                    parent.pid,
                    parent.command
                ));
                current_pid = Some(parent.ppid);
                level += 1;
            } else {
                break;
            }
        }

        content.push_str("\nChildren:\n");
        append_children_recursive(&mut content, &proc.children, 0);
    } else {
        content.push_str("Process: N/A\n");
    }

    let block = Block::default()
        .title(format!("Process Tree for PID {}", pid))
        .borders(Borders::ALL);
    let paragraph = Paragraph::new(content).block(block);
    f.render_widget(paragraph, area);
}

// Recursive helper function to append children
fn append_children_recursive(content: &mut String, children: &HashMap<i32, Process>, level: usize) {
    for child in children.values() {
        content.push_str(&format!(
            "{}- {} ({})\n",
            "  ".repeat(level),
            child.pid,
            child.command
        ));
        append_children_recursive(content, &child.children, level + 1);
    }
}



fn draw_empty_tree_view(f: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    let block = Block::default().title("Process Tree").borders(Borders::ALL);
    let paragraph = Paragraph::new("No process selected").block(block);
    f.render_widget(paragraph, area);
}


fn draw_help_section(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    sort_criteria: &SortCriteria,
    view_state: &ViewState,
) {
    let sort_label = match sort_criteria {
        SortCriteria::CPU => "Sorting by: CPU",
        SortCriteria::Memory => "Sorting by: Memory",
        SortCriteria::PID => "Sorting by: PID",
        SortCriteria::PR => "Sorting by: Priority",
    };
    let view_label = match view_state {
        ViewState::Processes => "View: Processes",
        ViewState::CrashTracking => "View: Crash Tracking",
        ViewState::ProcessTree => "View: Process Tree",
    };
    let help_text = format!(
        "{}\n{}\nKeys: q: Quit  t: Show tree  k: Kill s: Suspend  w: Wake  ←/→: Switch View  ↑/↓: Navigate    Sort by: c: CPU  m: Memory  p: PID  r: Priority",
        sort_label, view_label
    );
    let block = Block::default().title("Help").borders(Borders::ALL);
    let paragraph = Paragraph::new(help_text).block(block).style(
        Style::default()
    );
    f.render_widget(paragraph, area);
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
        .take(20)
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
                p.state.to_string(),
                p.threads.to_string(),
                p.priority.to_string(),
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
            Constraint::Length(4),   // State
            Constraint::Length(8),   // Threads
            Constraint::Length(6),   // Priority
            Constraint::Length(8),   // CPU
            Constraint::Length(10),  // Memory
            Constraint::Length(12),  // Time+
            Constraint::Min(20),     // Command
        ],
    )
    .header(Row::new(vec!["PID", "PPID", "USER", "ST", "THR", "PR", "%CPU", "MEM", "TIME+", "COMMAND"]))
    .block(Block::default().title("Processes").borders(Borders::ALL));

    f.render_widget(table, area);
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
