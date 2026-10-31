use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

pub const DEFAULT_DEADLINE: f64 = 300.0;
pub const DEFAULT_POLL_INTERVAL: f64 = 2.0;

pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (rojo 7.7.1)");

#[derive(Parser)]
#[command(name = "crucible", version = VERSION, about = "Tools for Crucible games.", arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand)]
pub enum Command {
    /// Compile the game with roblox-ts, then build each place in crucible.toml with Rojo.
    Build(crate::build::Args),
    /// Create a game and install its dependencies.
    Create(crate::create::Args),
    /// Watch and compile the game with roblox-ts, and serve one place to Studio with Rojo.
    Dev(crate::dev::Args),
    /// Print a Crucible guide as Markdown. Without a topic, list the topics.
    Docs {
        /// The topic to print.
        #[arg(value_parser = topics())]
        topic: Option<String>,
    },
    /// Call Roblox Open Cloud. One JSON document per command.
    #[command(after_help = AGENT_GUIDE)]
    Oc(Oc),
    /// Build and publish the places in a crucible.toml environment. One JSON document.
    Publish(crate::publish::Args),
    /// Upgrade crucible and @trioforge/crucible, then refresh template files this project has not edited.
    Upgrade(crate::upgrade::Args),
    /// Serve one Rojo project. crucible dev runs this in a child process.
    #[command(name = crate::dev::ROJO_SERVE, hide = true)]
    RojoServe {
        /// The Rojo project file.
        project: PathBuf,
        /// The port to listen on.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Run roblox-ts and stop it with every process it started. crucible dev runs this in a child process.
    #[command(name = crate::dev::WATCH_GUARD, hide = true)]
    WatchGuard {
        /// The command to run.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<std::ffi::OsString>,
    },
    /// Build one Rojo project. crucible build runs this in a child process.
    #[command(name = crate::build::ROJO_BUILD, hide = true)]
    RojoBuild {
        /// The Rojo project file.
        project: PathBuf,
        /// The place file to write.
        output: PathBuf,
    },
    /// Print this version's template and docs hashes as JSON. crucible upgrade runs this.
    #[command(name = crate::upgrade::SNAPSHOT, hide = true)]
    UpgradeSnapshot {
        /// The game's package name.
        name: String,
    },
}
fn topics() -> clap::builder::PossibleValuesParser {
    crate::docs::TOPICS
        .iter()
        .map(|topic| clap::builder::PossibleValue::new(topic.name).help(topic.summary()))
        .collect::<Vec<_>>()
        .into()
}
#[derive(Args)]
pub struct Oc {
    /// Select the credential profile to use.
    #[arg(long, global = true)]
    pub profile: Option<String>,
    /// Stop polling after this many seconds.
    #[arg(long,global=true,default_value_t=DEFAULT_DEADLINE,value_parser=seconds)]
    pub deadline: f64,
    /// Wait this many seconds between polls.
    #[arg(long,global=true,default_value_t=DEFAULT_POLL_INTERVAL,value_parser=seconds)]
    pub poll_interval: f64,
    /// Override retry delays in seconds.
    #[arg(long,global=true,value_parser=seconds)]
    pub retry_delay: Option<f64>,
    /// Write request and response details to stderr.
    #[arg(long, global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: OcCommand,
}
fn seconds(s: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 86400.0)
        .ok_or_else(|| "seconds must be between 0 and 86400".into())
}
#[derive(Subcommand)]
pub enum OcCommand {
    /// Find operations by words from their names and documentation.
    Find {
        /// Search words from operation names, paths, and documentation.
        #[arg(required = true)]
        query: Vec<String>,
        /// Return at most this many operations.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Show an operation contract and a ready-to-fill call.
    Describe {
        /// An operation name, or an HTTP method and URL.
        #[arg(required=true,num_args=1..=2)]
        operation: Vec<String>,
    },
    /// Call an operation by name or HTTP method and URL.
    Call(CallArgs),
    /// Upload and download assets.
    #[command(subcommand)]
    Asset(AssetCommand),
    /// Publish place files.
    #[command(subcommand)]
    Place(PlaceCommand),
    /// Run Luau and collect its results and logs.
    #[command(subcommand)]
    Luau(LuauCommand),
    /// Inspect credentials without printing their values.
    #[command(subcommand)]
    Auth(AuthCommand),
}
#[derive(Args)]
pub struct CallArgs {
    /// An operation name or method and URL, then name=value parameters.
    #[arg(required=true,num_args=1..)]
    pub words: Vec<String>,
    /// Send inline JSON, @file bytes, or @- from stdin.
    #[arg(long)]
    pub body: Option<String>,
    /// Add a multipart field as name=value or name=@file.
    #[arg(long = "part")]
    pub parts: Vec<String>,
    /// Follow pages and return one combined page document.
    #[arg(long)]
    pub all: bool,
    /// Stop after this many items and retain the continuation token.
    #[arg(long, requires = "all")]
    pub max_items: Option<usize>,
    /// Stop after this many pages and retain the continuation token.
    #[arg(long, requires = "all", default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..))]
    pub max_pages: u32,
    /// Return the first response without polling for completion.
    #[arg(long)]
    pub no_wait: bool,
}
#[derive(Subcommand)]
pub enum AuthCommand {
    /// Check the selected key and optionally its required scopes.
    Check {
        /// Check that the key has the scopes required by this operation.
        #[arg(long = "for")]
        for_operation: Option<String>,
    },
    /// List configured credential sources and key availability.
    Profiles,
}

#[derive(Subcommand)]
pub enum AssetCommand {
    /// Upload one asset file, then poll for its asset ID.
    Upload(crate::flows::upload::Args),
    /// Download an asset through a request that carries no API key.
    Download(crate::flows::download::Args),
}
#[derive(Subcommand)]
pub enum PlaceCommand {
    /// Publish a place file with the guarded retry policy.
    Publish(crate::flows::publish::Args),
}
#[derive(Subcommand)]
pub enum LuauCommand {
    /// Create a Luau task once, then poll and collect logs.
    Run(crate::flows::luau::Args),
}

const AGENT_GUIDE: &str = "Agent guide:
  find <words> -> describe <name> -> call <name> name=value
  Example: find data store entry; describe data-stores.get-entry
           call data-stores.get-entry universe_id=1 data_store_id=Profiles entry_id=player
  asset upload <file>       Create once, then poll for the asset ID.
  place publish <file>      Publish with the guarded retry policy.
  asset download <id>       Download to --out without sending the key to the CDN.
  luau run <file>           Create once, then collect results and logs.
  Keys: --profile > CRUCIBLE_OC_PROFILE > CRUCIBLE_OC_API_KEY > default_profile.
  Exit 0  Success. Read the document.
  Exit 2  usage. Fix invalid arguments, input, files, or read-result limits.
  Exit 3  auth. Missing key, HTTP 401/403, or failed key check. Fix the key or profile.
  Exit 4  rejected. Fix another 4xx request before repeating it.
  Exit 5  outcome_unknown. Check the remote resource before repeating a write.
  Exit 6  remote_failure. Inspect the operation error or Luau logs.
  Exit 7  unavailable. Unprocessed, unavailable, or unreadable response. Retry or resume a read.
  Exit 8  pending. Accepted work unfinished, or later poll or receipt failed. Run error.resume.";

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn undocumented(command: &clap::Command, path: &str) -> Vec<String> {
        let mut missing = Vec::new();
        if command.get_about().is_none_or(|about| about.to_string().trim().is_empty()) {
            missing.push(path.to_owned());
        }
        for argument in command.get_arguments() {
            if argument.get_help().is_none_or(|help| help.to_string().trim().is_empty()) {
                missing.push(format!("{path} {}", argument.get_id()));
            }
        }
        for child in command.get_subcommands() {
            missing.extend(undocumented(child, &format!("{path} {}", child.get_name())));
        }
        missing
    }

    #[test]
    fn version_names_the_locked_rojo() {
        let lock = include_str!("../../../Cargo.lock").replace("\r\n", "\n");
        let rojo = VERSION.strip_suffix(')').and_then(|v| v.split_once(" (rojo ")).map(|(_, rojo)| rojo).unwrap();
        let locked = format!("name = \"rojo\"\nversion = \"{rojo}\"\n");
        assert!(lock.contains(&locked), "Cargo.lock locks rojo {rojo}");
    }

    #[test]
    fn every_command_and_argument_has_help() {
        let mut command = Cli::command();
        command.build();
        let missing = undocumented(&command, "crucible");
        assert!(missing.is_empty(), "missing help descriptions: {missing:?}");
        let mut broken = Cli::command().mut_subcommand("oc", |oc| {
            oc.mut_subcommand("call", |call| call.mut_arg("body", |argument| argument.help("")))
        });
        broken.build();
        assert!(undocumented(&broken, "crucible").iter().any(|path| path == "crucible oc call body"));
    }

    fn oc_help() -> String {
        let mut command = Cli::command();
        command.build();
        command.find_subcommand_mut("oc").expect("oc is a subcommand").render_long_help().to_string()
    }

    #[test]
    fn oc_help_teaches_the_agent_loop_and_exit_codes() {
        let help = oc_help();
        for text in [
            "find <words>",
            "describe <name>",
            "call <name>",
            "data-stores.get-entry",
            "asset upload",
            "place publish",
            "asset download",
            "luau run",
            "CRUCIBLE_OC_API_KEY",
        ] {
            assert!(help.contains(text), "agent help must include {text}");
        }
        for exit in [0, 2, 3, 4, 5, 6, 7, 8] {
            assert!(help.contains(&format!("Exit {exit} ")), "agent help documents exit {exit}");
        }
    }

    #[test]
    fn agent_guide_aligns_flow_descriptions() {
        let help = oc_help();
        let columns: Vec<_> = help
            .lines()
            .filter(|line| {
                ["asset upload <file>", "place publish <file>", "asset download <id>", "luau run <file>"]
                    .iter()
                    .any(|command| line.trim_start().starts_with(command))
            })
            .map(|line| {
                line.find("> ").unwrap()
                    + 2
                    + line.split_once("> ").unwrap().1.chars().take_while(|character| *character == ' ').count()
            })
            .collect();
        assert_eq!(columns.len(), 4);
        assert!(columns.iter().all(|column| *column == columns[0]), "flow descriptions start at {columns:?}");
    }
}
