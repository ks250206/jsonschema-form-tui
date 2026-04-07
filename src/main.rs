use anyhow::Result;
use clap::{Parser, ValueEnum};
use jsonschema_form_tui::app::state::{AppMode, AppState};
use jsonschema_form_tui::ui::app::run_app;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Standard,
    Editor,
}

impl From<ModeArg> for AppMode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::Standard => AppMode::Standard,
            ModeArg::Editor => AppMode::Editor,
        }
    }
}

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[arg(short, long)]
    schema: Option<String>,
    #[arg(short, long)]
    output: Option<String>,
    #[arg(long, value_enum, default_value_t = ModeArg::Standard)]
    mode: ModeArg,
}

fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let mut state = AppState::new_with_mode(cli.mode.into());

    if let Some(schema) = cli.schema {
        state.set_schema_source(schema)?;
    }

    if let Some(output) = cli.output {
        state.schema_path.output_path = output;
    }

    run_app(state)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .try_init();
}
