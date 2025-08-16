use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "devenv", version, about = "Simple dev environment manager", long_about = None)]
pub struct Cli {
    /// Print subprocess output and more logging
    #[arg(global = true, short, long)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize a dev environment in the given project directory
    Init { path: Option<PathBuf> },
    /// List running dev environments
    List,
    /// Start the named environment
    Start(StartArgs),
    /// Stop the named environment (or infer from CWD)
    Stop { name: Option<String> },
    /// Remove the environment container and unregister it (or infer from CWD)
    Remove { name: Option<String> },
    /// Attach an interactive shell to the environment (or infer from CWD)
    Attach { name: Option<String> },
    /// Restart the environment: stop if running, then start (accepts same flags as start)
    Restart(StartArgs),
    /// Build the environment image without starting a container
    Build(BuildArgs),
}

#[derive(Args, Debug)]
pub struct StartArgs {
    /// Environment name (optional; inferred from devenv.toml in CWD when omitted)
    pub name: Option<String>,
    /// Open the project in an IDE after start. Optional command, defaults to 'zed'.
    #[arg(long, value_name = "CMD", num_args = 0..=1, default_missing_value = "zed")]
    pub open: Option<String>,
    /// Attach an interactive shell after starting the environment
    #[arg(long)]
    pub attach: bool,
    /// Rebuild the Dockerfile from devenv.toml before building
    #[arg(long)]
    pub rebuild: bool,
    /// Skip building the image if present
    #[arg(long)]
    pub no_build: bool,
}

#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Environment name (optional; inferred from devenv.toml in CWD when omitted)
    pub name: Option<String>,
    /// Rebuild the Dockerfile from devenv.toml before building
    #[arg(long)]
    pub rebuild: bool,
    /// Always pull newer base layers
    #[arg(long)]
    pub pull: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_start_open_default_and_flags() {
        let cli = Cli::parse_from(["devenv", "start", "--open"]);
        match cli.command {
            Commands::Start(args) => {
                assert_eq!(args.open.as_deref(), Some("zed"));
                assert!(!args.attach);
                assert!(!args.rebuild);
                assert!(!args.no_build);
                assert!(args.name.is_none());
            }
            _ => panic!("expected start"),
        }
    }

    #[test]
    fn parses_start_open_explicit_command() {
        let cli = Cli::parse_from(["devenv", "start", "--open", "code", "myenv"]);
        match cli.command {
            Commands::Start(args) => {
                assert_eq!(args.open.as_deref(), Some("code"));
                assert_eq!(args.name.as_deref(), Some("myenv"));
            }
            _ => panic!("expected start"),
        }
    }

    #[test]
    fn parses_build_with_pull_and_rebuild() {
        let cli = Cli::parse_from(["devenv", "build", "--pull", "--rebuild", "proj"]);
        match cli.command {
            Commands::Build(args) => {
                assert!(args.pull);
                assert!(args.rebuild);
                assert_eq!(args.name.as_deref(), Some("proj"));
            }
            _ => panic!("expected build"),
        }
    }
}
