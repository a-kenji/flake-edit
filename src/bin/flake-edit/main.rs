use clap::Parser;
use color_eyre::eyre;
use flake_edit::cli::CliArgs;

mod log;

fn main() -> eyre::Result<()> {
    let args = CliArgs::parse();

    // Hide internal source locations from error output
    let builder = color_eyre::config::HookBuilder::new()
        .display_location_section(false)
        .display_env_section(false);

    let builder = if std::env::var("NO_COLOR").is_ok() {
        builder.theme(color_eyre::config::Theme::new())
    } else {
        builder
    };
    builder.install()?;

    log::init().ok();
    tracing::debug!("Cli args: {args:?}");

    flake_edit::app::run(args)?;

    Ok(())
}
