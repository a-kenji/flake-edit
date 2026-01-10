use std::{fs, path::PathBuf};

use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, fmt};

const LOG_ENV: &str = "FE_LOG";

/// Configuration of logging
pub fn init_logging(log_file: Option<PathBuf>) -> Result<(), std::io::Error> {
    // Check if running in CI environment - if so, log to stderr instead of file
    let log_to_stdout = std::env::var("CI").is_ok();

    if log_to_stdout {
        // Log to stderr (so it doesn't interfere with stdout output)
        let subscriber = fmt::Subscriber::builder()
            .with_max_level(Level::TRACE)
            .with_writer(std::io::stderr)
            .with_thread_ids(false)
            .with_ansi(true)
            .with_line_number(true);

        if let Ok(env_filter) = EnvFilter::try_from_env(LOG_ENV) {
            let subscriber = subscriber.with_env_filter(env_filter).finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("setting default subscriber failed");
        } else {
            let subscriber = subscriber.finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("setting default subscriber failed");
        }
    } else {
        // Log to file
        let log_file = match log_file {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::File::create(path)?
            }
            None => {
                // If no file specified and not logging to stdout, do nothing
                return Ok(());
            }
        };

        let subscriber = fmt::Subscriber::builder()
            .with_max_level(Level::TRACE)
            .with_writer(std::sync::Mutex::new(log_file))
            .with_thread_ids(false)
            .with_ansi(true)
            .with_line_number(true);

        if let Ok(env_filter) = EnvFilter::try_from_env(LOG_ENV) {
            let subscriber = subscriber.with_env_filter(env_filter).finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("setting default subscriber failed");
        } else {
            let subscriber = subscriber.finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("setting default subscriber failed");
        }
    }

    Ok(())
}

pub fn init() -> Result<(), std::io::Error> {
    init_logging(Some("/tmp/flake-edit/flake-edit.log".into()))
}
