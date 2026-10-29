// SPDX-License-Identifier: MIT OR Apache-2.0

mod backup;
mod commands;
mod docker;
mod git;
mod runtime;
mod ssh;
mod templates;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "dshctl",
    version,
    about = "Deploy and operate the DSH Docker runtime",
    long_about = "A single static CLI for the DSH Docker runtime. It initializes the project, \
                  manages persistent state, pulls or builds images, manages Basic Auth users, \
                  diagnoses the deployment, and creates backups.\n\n\
                  Documentation: https://github.com/reina0721/dsh-docker"
)]
pub struct Cli {
    /// Project directory (defaults to the current directory).
    #[arg(long, global = true, default_value = ".")]
    pub dir: PathBuf,
    /// Print Docker commands before running them.
    #[arg(long, global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Create .env, compose.yaml (when missing), and the persistent runtime tree.
    Init(InitArgs),
    /// Preflight, authenticate, prepare the image, and start the deployment.
    Deploy(DeployArgs),
    /// Start the deployment without pulling or building.
    Up,
    /// Stop and remove containers and the project network.
    Down,
    /// Stop the running container without removing it.
    Stop,
    /// Restart the running container.
    Restart,
    /// Show service, HTTP, auth, and image status.
    Status(StatusArgs),
    /// Run deployment diagnostics.
    Doctor(DoctorArgs),
    /// Follow container logs.
    Logs(LogsArgs),
    /// Open a shell inside the DSH container.
    Shell,
    /// Execute a command inside the DSH container.
    Exec(ExecArgs),
    /// Pull the latest image and recreate the container.
    Update(UpdateArgs),
    /// Upgrade DSH (@deepseek-ai/dsh) to the latest upstream release.
    Upgrade(UpgradeArgs),
    /// Replace this dshctl binary from GitHub Releases.
    SelfUpdate(SelfUpdateArgs),
    /// Rebuild the image locally and recreate the container.
    Rebuild(RebuildArgs),
    /// Manage Basic Auth users.
    User(UserArgs),
    /// Manage persistent SSH keys for Git hosting.
    Ssh(SshArgs),
    /// Configure the persistent developer Git identity.
    Git(GitArgs),
    /// Create a tar.gz backup of .env and the runtime directory.
    Backup(BackupArgs),
    /// Restore a backup created by this tool.
    Restore(RestoreArgs),
    /// Inspect or pull the runtime image.
    Image(ImageArgs),
    /// Validate or show configuration.
    Config(ConfigArgs),
    /// Migrate a v1 deployment to v2, or migrate this project to a new target directory.
    Migrate(MigrateArgs),
    /// Generate shell completions.
    Completion(CompletionArgs),
    /// Print version and target information.
    Version,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Overwrite compose.yaml/.env.example even if they already exist.
    #[arg(long)]
    pub force: bool,
    /// Back up and refresh generated templates.
    #[arg(long)]
    pub refresh: bool,
}

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Image delivery: auto, pull, or build.
    #[arg(long)]
    pub mode: Option<String>,
    /// Mirror policy: global, cn, or off.
    #[arg(long)]
    pub mirror: Option<String>,
    /// Use the slim runtime image/target.
    #[arg(long)]
    pub slim: bool,
    /// Do not prompt; fail if authentication users are missing.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Repair runtime permissions and missing directories.
    #[arg(long)]
    pub fix: bool,
    /// Also probe GHCR and the layer CDN.
    #[arg(long)]
    pub network: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Follow log output.
    #[arg(long, short)]
    pub follow: bool,
    /// Number of lines to show.
    #[arg(long)]
    pub tail: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Command and arguments (use `--` to separate).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    pub command: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Pull the source checkout with git before recreating.
    #[arg(long)]
    pub source: bool,
    /// Back up .env and the runtime directory before updating.
    #[arg(long)]
    pub backup: bool,
}

#[derive(Debug, Args)]
pub struct SelfUpdateArgs {
    /// Only check for a newer release.
    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    /// Specific DSH version to install (defaults to latest upstream on npm).
    #[arg(long)]
    pub version: Option<String>,
    /// Only check for newer upstream DSH version without upgrading.
    #[arg(long)]
    pub check: bool,
    /// Rebuild the image instead of performing an in-container live upgrade.
    #[arg(long)]
    pub rebuild: bool,
    /// Do not prompt for confirmation.
    #[arg(long, short)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct RebuildArgs {
    /// Build the slim target.
    #[arg(long)]
    pub slim: bool,
    /// Do not prompt; fail if authentication users are missing.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct UserArgs {
    #[command(subcommand)]
    pub command: UserCommands,
}

#[derive(Debug, Subcommand)]
pub enum UserCommands {
    /// Create a user or change an existing user's password.
    Add(UserAddArgs),
    /// List usernames.
    List(UserListArgs),
    /// Remove a user.
    Remove(UserRemoveArgs),
}

#[derive(Debug, Args)]
pub struct UserAddArgs {
    pub name: String,
    /// bcrypt cost (4-31; defaults to AUTH_BCRYPT_COST).
    #[arg(long)]
    pub cost: Option<u32>,
    /// Read the password from stdin.
    #[arg(long)]
    pub password_stdin: bool,
    /// Read the password from a file.
    #[arg(long)]
    pub password_file: Option<PathBuf>,
    /// Generate and print a random password.
    #[arg(long)]
    pub generate: bool,
    /// Allow weak/short passwords below AUTH_MIN_PASSWORD_LENGTH.
    #[arg(long)]
    pub insecure_password: bool,
}

#[derive(Debug, Args)]
pub struct UserListArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UserRemoveArgs {
    pub name: String,
    /// Do not ask for confirmation.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct SshArgs {
    #[command(subcommand)]
    pub command: SshCommands,
}

#[derive(Debug, Subcommand)]
pub enum SshCommands {
    /// List keys and fingerprints (never private key material).
    List(SshListArgs),
    /// Generate a new key in the persistent SSH directory.
    Generate(SshGenerateArgs),
    /// Import an existing private key (and optional public key).
    Import(SshImportArgs),
    /// Remove a keypair.
    Remove(SshRemoveArgs),
    /// Test an SSH connection to a Git host.
    Test(SshTestArgs),
}

#[derive(Debug, Args)]
pub struct SshListArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SshGenerateArgs {
    /// Key name (defaults to id_ed25519/id_rsa/id_ecdsa).
    pub name: Option<String>,
    /// Key type: ed25519, rsa, or ecdsa.
    #[arg(long = "type", default_value = "ed25519")]
    pub key_type: String,
    /// Public key comment.
    #[arg(long)]
    pub comment: Option<String>,
    /// Do not prompt for a passphrase (unattended use).
    #[arg(long)]
    pub no_passphrase: bool,
    /// Replace an existing keypair.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct SshImportArgs {
    /// Path to the private key.
    pub private_key: PathBuf,
    /// Optional public key path.
    #[arg(long)]
    pub public_key: Option<PathBuf>,
    /// Destination name (defaults to the source filename).
    #[arg(long)]
    pub name: Option<String>,
    /// Replace an existing keypair.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct SshRemoveArgs {
    /// Key name.
    pub name: String,
    /// Do not ask for confirmation.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct SshTestArgs {
    /// github, gitlab, gitee, a hostname, or user@host.
    pub host: String,
    /// Key name to use (defaults to the first private key).
    #[arg(long)]
    pub key: Option<String>,
}

#[derive(Debug, Args)]
pub struct GitArgs {
    #[command(subcommand)]
    pub command: GitCommands,
}

#[derive(Debug, Subcommand)]
pub enum GitCommands {
    /// Set user.name/user.email and optionally core.sshCommand.
    Setup(GitSetupArgs),
    /// Show the persistent Git config.
    Show,
}

#[derive(Debug, Args)]
pub struct GitSetupArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub email: Option<String>,
    /// SSH key name to use for Git (sets core.sshCommand).
    #[arg(long)]
    pub key: Option<String>,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    /// Output path (defaults to ./dsh-docker-backup-<unix>.tar.gz).
    pub output: Option<PathBuf>,
    /// Stop the container while archiving for a consistent snapshot.
    #[arg(long)]
    pub quiesce: bool,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    /// Archive to restore.
    pub archive: PathBuf,
    /// Confirm the destructive restore.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct ImageArgs {
    #[command(subcommand)]
    pub command: ImageCommands,
}

#[derive(Debug, Subcommand)]
pub enum ImageCommands {
    /// Check registry, layer CDN, and manifest reachability.
    Check(ImageCheckArgs),
    /// Pull the configured image, trying mirrors when requested.
    Pull(ImagePullArgs),
}

#[derive(Debug, Args)]
pub struct ImageCheckArgs {
    /// Include network probes (default true; kept for symmetry).
    #[arg(long, default_value_t = true)]
    pub network: bool,
}

#[derive(Debug, Args)]
pub struct ImagePullArgs {
    /// Mirror policy: global, cn, or off.
    #[arg(long)]
    pub mirror: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Validate .env and typed configuration.
    Validate,
    /// Show configuration with secrets redacted.
    Show(ConfigShowArgs),
}

#[derive(Debug, Args)]
pub struct ConfigShowArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct MigrateArgs {
    /// Target directory to migrate this installation to (e.g. /workapps/dsh-docker).
    #[arg(long, value_name = "DIR")]
    pub to: Option<PathBuf>,
    /// Target directory to migrate this installation to (positional alias).
    #[arg(value_name = "DEST")]
    pub dest: Option<PathBuf>,
    /// Keep source directory after copying (default is to move/relocate).
    #[arg(long)]
    pub copy: bool,
    /// Overwrite or merge if target directory already exists.
    #[arg(long)]
    pub force: bool,
    /// Do not automatically deploy/start the service in the target directory.
    #[arg(long)]
    pub no_deploy: bool,
    /// Print planned actions without changing files.
    #[arg(long)]
    pub dry_run: bool,
    /// Do not prompt for confirmation.
    #[arg(long, short)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

fn main() {
    let cli = Cli::parse();
    if let Err(error) = commands::run(cli) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
