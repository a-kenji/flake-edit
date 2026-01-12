use clap::Parser;
use color_eyre::eyre;
use flake_edit::cli::CliArgs;

mod log;

fn main() -> eyre::Result<()> {
    let args = CliArgs::parse();

    if std::env::var("NO_COLOR").is_err() {
        color_eyre::install()?;
    } else {
        color_eyre::config::HookBuilder::new()
            .theme(color_eyre::config::Theme::new())
            .install()?;
    }

    log::init().ok();
    tracing::debug!("Cli args: {args:?}");

    flake_edit::app::run(args)?;

    Ok(())
}
