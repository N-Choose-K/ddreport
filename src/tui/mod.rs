//! Interactive terminal UI: browse drives, run self-tests, export PDF reports.

mod app;
mod device;
mod export;
#[cfg(test)]
mod render_tests;
mod ui;
mod worker;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

pub use device::{Device, DeviceSource};
pub use export::{export_filename, sanitize};

use app::App;
use worker::{Job, Msg};

const TICK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Default)]
pub struct TuiOptions {
    /// Saved `smartctl -x --json` files; when non-empty no hardware is touched.
    pub demo_files: Vec<PathBuf>,
    pub out_dir: PathBuf,
    /// smartctl `-d` override; None = auto-probe.
    pub device_type: Option<String>,
}

pub fn run(opts: TuiOptions) -> anyhow::Result<()> {
    let mut terminal = ratatui::try_init()?;
    silence_worker_panics();
    let result = event_loop(&mut terminal, App::new(&opts));
    ratatui::restore();
    result
}

/// Worker panics are caught and shown in the UI; only main-thread panics may
/// tear down the terminal (ratatui's hook) and print.
fn silence_worker_panics() {
    let restore_and_report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !worker::is_worker_thread() {
            restore_and_report(info);
        }
    }));
}

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let spawn = |jobs: Vec<Job>| jobs.into_iter().for_each(|j| worker::spawn(j, tx.clone()));
    spawn(app.start());
    let mut last_tick = Instant::now();
    while !app.should_quit {
        terminal.draw(|f| ui::draw(f, &app))?;
        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            spawn(app.on_key(key));
        }
        while let Ok(msg) = rx.try_recv() {
            spawn(app.on_msg(msg, Instant::now()));
        }
        if last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
            spawn(app.on_tick(last_tick));
        }
    }
    Ok(())
}
