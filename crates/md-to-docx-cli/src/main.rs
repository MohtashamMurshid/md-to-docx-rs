mod args;
mod cli;
mod json_options;
mod locale_sort;

use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() -> ExitCode {
    let abort = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::signal::SIGINT, Arc::clone(&abort));
    let _ = signal_hook::flag::register(signal_hook::consts::signal::SIGTERM, Arc::clone(&abort));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut hooks = cli::CliHooks::stdio();
    let code = cli::run_cli(&args, &mut hooks, &abort);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
