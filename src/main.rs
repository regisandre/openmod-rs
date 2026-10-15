use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use openmod::config::{Config, DEFAULT_CONFIG};
use openmod::engine::Engine;
use openmod::files::write_atomic;
use openmod::keys::State;
use std::io::Read;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "openmod",
    version,
    about = "Lightweight self-hosted image and text moderation"
)]
struct Cli {
    /// Path to the configuration file.
    #[arg(
        short,
        long,
        global = true,
        env = "OPENMOD_CONFIG",
        default_value = "openmod.toml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the API and the admin interface. Writes a default configuration
    /// first if there is none.
    Serve,
    /// Download the model files of the enabled tools.
    Fetch {
        /// Every tool of the configuration, enabled or not.
        #[arg(long)]
        all: bool,
    },
    /// Print a new admin token. The old one, and every session opened with
    /// it, stop working.
    AdminToken,
    /// Check one image file and print the JSON report.
    Image {
        file: PathBuf,
        #[arg(short, long, default_value = "default")]
        workflow: String,
    },
    /// Check a text (use `-` to read stdin) and print the JSON report.
    Text {
        text: String,
        #[arg(short, long, default_value = "default")]
        workflow: String,
    },
    /// Validate the configuration without loading any model.
    Validate,
    /// Evaluate a labelled corpus and write every score for calibration.
    Eval {
        #[arg(long)]
        images: Option<PathBuf>,
        #[arg(long)]
        texts: Option<PathBuf>,
        #[arg(short, long, default_value = "default")]
        workflow: String,
        #[arg(long, default_value = "eval-scores.jsonl")]
        dump: PathBuf,
    },
}

/// For the one-shot commands: every enabled model must load.
fn ready_engine(config: Config) -> Result<Engine> {
    let engine = Engine::load(config, None);
    engine.require_ready()?;
    Ok(engine)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "openmod=info,warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    if matches!(cli.command, Command::Serve) && !cli.config.exists() {
        write_atomic(&cli.config, DEFAULT_CONFIG.as_bytes(), false)?;
        tracing::info!(
            "wrote the default configuration to {}",
            cli.config.display()
        );
    }
    let config = Config::load(&cli.config)?;

    match cli.command {
        Command::Validate => {
            println!(
                "{} is valid: {} models ({} enabled), {} workflows",
                cli.config.display(),
                config.models.len(),
                config.models.values().filter(|m| m.enabled).count(),
                config.workflows.len()
            );
            for rule in config.inactive_rules() {
                println!("  not checked, its models are disabled: {rule}");
            }
        }
        Command::Fetch { all } => openmod::download::fetch_command(&config, all)?,
        Command::AdminToken => {
            let path = &config.server.state_file;
            let mut state = State::load(path)?;
            let token = state.rotate_admin_token();
            state.save(path)?;
            println!("{token}");
        }
        Command::Serve => {
            let engine = Engine::load(config, None);
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(openmod::server::serve(cli.config, engine))?
        }
        Command::Image { file, workflow } => {
            let report = ready_engine(config)?.check_image(&std::fs::read(file)?, &workflow)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Text { text, workflow } => {
            let text = if text == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                text
            };
            let report = ready_engine(config)?.check_text(&text, &workflow)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Eval {
            images,
            texts,
            workflow,
            dump,
        } => {
            if images.is_none() && texts.is_none() {
                bail!("give --images, --texts or both");
            }
            openmod::eval::run(
                &ready_engine(config)?,
                images.as_deref(),
                texts.as_deref(),
                &workflow,
                &dump,
            )?
        }
    }
    Ok(())
}
