//! The `meson-jll` command line tool.
//!
//! This binary only parses arguments and prints results. Every actual
//! behavior lives in the `meson_jll` library, so it can be tested directly
//! without going through a subprocess. See the library's crate
//! documentation (`cargo doc`) for the full user guide.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use meson_jll::progress::Progress;
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

    /// Regenerate every wrap in the lockfile, for example after a fresh
    /// checkout where only the committed lockfile is present.
    Sync {
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
        Command::Sync { subprojects_dir } => run_sync(&subprojects_dir),
        Command::Info { name } => run_info(&name),
        Command::Status { subprojects_dir } => run_status(&subprojects_dir),
        Command::Update {
            name,
            subprojects_dir,
        } => run_update(name.as_deref(), &subprojects_dir),
    }
}

fn run_list() -> anyhow::Result<()> {
    let names = registry::list_jll_packages()?;
    for name in names {
        println!("{name}");
    }
    Ok(())
}

fn run_search(term: &str) -> anyhow::Result<()> {
    let names = registry::search_jll_packages(term)?;
    for name in names {
        println!("{name}");
    }
    Ok(())
}

/// Drives a spinner and prints a timed "<Verb> N packages in <elapsed>"
/// summary per phase (resolving, then writing), the same way `uv` reports
/// its own install phases, from the [`Progress`] events a library call
/// reports as it runs.
///
/// A [`Progress::Resolved`] event marks the boundary between the two
/// phases: it prints the resolve phase's summary immediately (suspending
/// the spinner so the line is not overwritten), then starts timing the
/// write phase that follows. [`Self::finish`] prints whichever phase was
/// running when the library call returned (the only phase there is, for an
/// offline `--locked` regeneration that never resolves anything).
struct PhaseReporter {
    bar: ProgressBar,
    phase_start: Instant,
}

impl PhaseReporter {
    fn new(initial_message: &'static str) -> Self {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .expect("spinner template is valid"),
        );
        bar.enable_steady_tick(Duration::from_millis(80));
        bar.set_message(initial_message);
        Self {
            bar,
            phase_start: Instant::now(),
        }
    }

    fn report(&mut self, event: Progress) {
        match event {
            Progress::Resolving(name) => {
                self.bar
                    .set_message(format!("Resolving versions... ({name})"));
            }
            Progress::Resolved { count } => {
                let elapsed = self.phase_start.elapsed();
                self.bar.suspend(|| {
                    println!("Resolved {count} packages in {}", format_elapsed(elapsed))
                });
                self.phase_start = Instant::now();
                self.bar.set_message("Generating wraps...");
            }
            Progress::Writing(name) => {
                self.bar
                    .set_message(format!("Generating wraps... ({name})"));
            }
        }
    }

    /// Prints the final phase's summary line and clears the spinner.
    /// `verb` names what the whole operation just did (for example
    /// `"Installed"`).
    fn finish(self, verb: &str, count: usize) {
        let elapsed = self.phase_start.elapsed();
        self.bar.finish_and_clear();
        println!("{verb} {count} packages in {}", format_elapsed(elapsed));
    }
}

/// Formats a duration the way `uv` does: milliseconds under a second,
/// seconds with two decimal places above it.
fn format_elapsed(elapsed: Duration) -> String {
    if elapsed.as_secs() >= 1 {
        format!("{:.2}s", elapsed.as_secs_f64())
    } else {
        format!("{}ms", elapsed.as_millis())
    }
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
        let mut reporter = PhaseReporter::new("Regenerating wraps...");
        let mut on_progress = |event: Progress| reporter.report(event);
        let installed = install::install_locked(subprojects_dir, force, &mut on_progress)?;
        reporter.finish("Installed", installed.len());
        installed
    } else {
        let name = name.expect("clap requires a name unless --locked is set");
        let mut reporter = PhaseReporter::new("Resolving versions...");
        let mut on_progress = |event: Progress| reporter.report(event);
        let installed =
            install::install(name, version, url, subprojects_dir, force, &mut on_progress)?;
        reporter.finish("Installed", installed.len());
        installed
    };
    for (name, version) in installed {
        println!("Installed {name} {version}");
    }
    Ok(())
}

fn run_sync(subprojects_dir: &Path) -> anyhow::Result<()> {
    // Sync is `install --locked` under a friendlier name: it regenerates
    // every wrap straight from the committed lockfile, force-overwriting
    // so the subprojects directory ends up matching the lock exactly,
    // whatever state (or absence) it started in.
    let mut reporter = PhaseReporter::new("Regenerating wraps...");
    let mut on_progress = |event: Progress| reporter.report(event);
    let installed = install::install_locked(subprojects_dir, true, &mut on_progress)?;
    reporter.finish("Synced", installed.len());
    for (name, version) in installed {
        println!("Synced {name} {version}");
    }
    Ok(())
}

fn run_info(name: &str) -> anyhow::Result<()> {
    registry::canonical_bare_name(name)?;
    let (owner, repo) = registry::resolve(name);
    let tags = registry::list_tags(&owner, &repo)?;
    for (tag, _sha) in tags {
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
    let mut reporter = PhaseReporter::new("Resolving versions...");
    let mut on_progress = |event: Progress| reporter.report(event);
    let installed = install::update(name, subprojects_dir, true, &mut on_progress)?;
    reporter.finish("Updated", installed.len());
    for (name, version) in installed {
        println!("updated {name} to {version}");
    }
    Ok(())
}
