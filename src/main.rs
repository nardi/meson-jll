//! The `meson-jll` command line tool.
//!
//! This binary only parses arguments and prints results. Every actual
//! behavior lives in the `meson_jll` library, so it can be tested directly
//! without going through a subprocess. See the library's crate
//! documentation (`cargo doc`) for the full user guide.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use meson_jll::{install, registry, status};

#[derive(Parser)]
#[command(
    name = "meson-jll",
    about = "Generate Meson wraps from Julia JLL packages",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List every JLL package published under JuliaBinaryWrappers.
    List,

    /// Search for JLL packages whose name contains a term.
    Search { term: String },

    /// Generate a JLL's wrap set and write it into subprojects/.
    Install {
        /// The JLL package name, bare or with its _jll suffix. Not needed
        /// with --locked, which regenerates every already-locked package.
        #[arg(required_unless_present = "locked")]
        name: Option<String>,
        /// The JLL release to install, for example 7.12.1+0. Defaults to
        /// the latest available version.
        version: Option<String>,
        /// Read the package's metadata from this git URL or local path
        /// instead of the JuliaBinaryWrappers organization.
        #[arg(long)]
        url: Option<String>,
        /// Overwrite files that already exist.
        #[arg(long)]
        force: bool,
        /// Regenerate every wrap straight from the existing lockfile,
        /// without resolving anything or checking for newer versions.
        #[arg(long)]
        locked: bool,
        /// The subprojects directory to write into.
        #[arg(long, default_value = "subprojects")]
        subprojects_dir: PathBuf,
    },

    /// Show the versions available for a JLL package.
    Info { name: String },

    /// Show installed JLL wraps in this project and whether newer versions
    /// exist.
    Status {
        /// The subprojects directory to scan.
        #[arg(long, default_value = "subprojects")]
        subprojects_dir: PathBuf,
    },

    /// Regenerate an installed JLL's wrap set to its latest version. Every
    /// installed JLL is updated when no name is given.
    Update {
        /// The JLL package name to update. Every installed JLL is updated
        /// when this is omitted.
        name: Option<String>,
        /// The subprojects directory to update.
        #[arg(long, default_value = "subprojects")]
        subprojects_dir: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => run_list(),
        Command::Search { term } => run_search(&term),
        Command::Install {
            name,
            version,
            url,
            force,
            locked,
            subprojects_dir,
        } => run_install(
            name.as_deref(),
            version.as_deref(),
            url.as_deref(),
            force,
            locked,
            &subprojects_dir,
        ),
        Command::Info { name } => run_info(&name),
        Command::Status { subprojects_dir } => run_status(&subprojects_dir),
        Command::Update {
            name,
            subprojects_dir,
        } => run_update(name.as_deref(), &subprojects_dir),
    }
}

fn run_list() -> anyhow::Result<()> {
    let mut names = registry::list_jll_packages()?;
    names.sort();
    for name in names {
        println!("{name}");
    }
    Ok(())
}

fn run_search(term: &str) -> anyhow::Result<()> {
    let mut names = registry::search_jll_packages(term)?;
    names.sort();
    for name in names {
        println!("{name}");
    }
    Ok(())
}

fn run_install(
    name: Option<&str>,
    version: Option<&str>,
    url: Option<&str>,
    force: bool,
    locked: bool,
    subprojects_dir: &Path,
) -> anyhow::Result<()> {
    let installed = if locked {
        install::install_locked(subprojects_dir, force)?
    } else {
        let name = name.expect("clap requires a name unless --locked is set");
        install::install(name, version, url, subprojects_dir, force)?
    };
    for (name, version) in installed {
        println!("Installed {name} {version}");
    }
    Ok(())
}

fn run_info(name: &str) -> anyhow::Result<()> {
    let (owner, repo) = registry::resolve(name);
    let tags = registry::list_tags(&owner, &repo)?;
    for tag in tags {
        if let Some(version) = registry::version_from_tag(&tag) {
            println!("{version}");
        }
    }
    Ok(())
}

fn run_status(subprojects_dir: &Path) -> anyhow::Result<()> {
    let installed = status::installed_packages(subprojects_dir)?;
    if installed.is_empty() {
        println!("no JLL wraps installed in {}", subprojects_dir.display());
        return Ok(());
    }
    for package in installed {
        let (owner, repo) = registry::resolve(&package.name);
        match registry::latest_version(&owner, &repo) {
            Ok(latest) if latest == package.version => {
                println!("{} {} (up to date)", package.name, package.version);
            }
            Ok(latest) => {
                println!("{} {} (latest: {latest})", package.name, package.version);
            }
            Err(error) => {
                println!(
                    "{} {} (could not check latest: {error})",
                    package.name, package.version
                );
            }
        }
    }
    Ok(())
}

fn run_update(name: Option<&str>, subprojects_dir: &Path) -> anyhow::Result<()> {
    let installed = install::update(name, subprojects_dir, true)?;
    for (name, version) in installed {
        println!("updated {name} to {version}");
    }
    Ok(())
}
