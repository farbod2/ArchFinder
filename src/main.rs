mod actions;
mod app;
mod config;
mod model;
mod scan;
mod ui;

use app::{App, TerminalEvent};
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::time::Duration;

/// ArchFinder — find and reclaim unused files and folders on Arch Linux.
#[derive(Parser, Debug)]
#[command(name = "archfinder", version, about, long_about = None)]
struct Args {
    /// Mode: 0 = stale files, 1 = junk & leftovers, 2 = custom paths.
    #[arg(short, long, default_value_t = 0)]
    mode: usize,

    /// Start scanning immediately on launch.
    #[arg(short = 'R', long)]
    scan: bool,

    /// Path to scan in stale mode (defaults to your home directory).
    #[arg(short, long)]
    path: Option<String>,

    /// Scan results as JSON (no TUI).
    #[arg(long)]
    json: bool,

    /// Minimum age in days (overrides config).
    #[arg(long)]
    min_age: Option<u64>,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let (mut config, cfg_path) = config::Config::load();
    if let Some(age) = args.min_age {
        config.min_age_days = age;
    }

    if args.json {
        return run_json(args, config);
    }

    let mut terminal = init_tui()?;
    let res = run_tui(args, config, cfg_path, &mut terminal);
    restore_tui()?;
    res
}

fn init_tui() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_tui() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

fn run_tui(
    args: Args,
    config: config::Config,
    cfg_path: Option<std::path::PathBuf>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> io::Result<()> {
    let mut app = App::new(config);
    app.mode = args.mode.min(2);
    if let Some(p) = cfg_path {
        app.status = format!("config: {} — press 1/2/3 then r to scan", p.display());
    }
    if args.scan {
        app.start_scan();
    }

    loop {
        if let Err(e) = terminal.draw(|f| ui::draw(f, &mut app)) {
            break Err(e);
        }

        // drive scan progress even between events
        app.poll_scan();

        match App::next_event(Duration::from_millis(100)) {
            Some(TerminalEvent::Key(k)) => app.handle_key(k),
            Some(TerminalEvent::Mouse) => {}
            Some(TerminalEvent::Resize) => {}
            None => {}
        }

        if app.should_quit {
            break Ok(());
        }
    }
}

fn run_json(args: Args, config: config::Config) -> io::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let req = scan::ScanRequest { mode: args.mode.min(2), config, path: args.path.clone() };
    scan::spawn_scan(req, tx);
    loop {
        match rx.recv() {
            Ok(scan::ScanMsg::Done(Ok(entries))) => {
                let out: Vec<_> = entries
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "path": e.path.display().to_string(),
                            "kind": e.kind.label(),
                            "size": e.size,
                            "age_days": e.age_days(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                break;
            }
            Ok(scan::ScanMsg::Done(Err(e))) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    Ok(())
}
