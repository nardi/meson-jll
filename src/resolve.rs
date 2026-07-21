//! Resolving a JLL package and its transitive JLL dependencies to one mutually
//! compatible set of versions.
//!
//! Julia JLLs declare version bounds on their JLL dependencies in a `[compat]`
//! section of `Project.toml` (see
//! [`crate::jll::project::ProjectToml::compat_for_dependency`]). [`resolve`] is
//! used to find one version per package that satisfies every bound in the whole
//! dependency graph, the way Julia's own `Pkg` resolver keeps an environment
//! consistent. See [`crate::internals`], "Resolving versions", for the fuller
//! picture, and [`crate::lockfile`] for where the result of a resolve is
//! recorded.
//!
//! [`resolve`] is a **fixed-point computation**, not a backtracking or SAT
//! solver. It repeatedly resolves each package to the highest available version
//! satisfying every compat range accumulated against it so far, and repeats
//! until a full pass changes nothing. Constraints only ever accumulate over the
//! course of a resolve and are never retracted, which is the one simplification
//! compared to a backtracking solver: the result can be slightly more
//! conservative than strictly necessary (a constraint from a branch that later
//! turns out irrelevant still applies), but it is never wrong, because a
//! version satisfying a superset of the real constraints always satisfies the
//! real ones too. This is enough for JLL dependency graphs specifically because
//! they are shallow and generated mechanically from a single upstream build, so
//! genuinely conflicting compat ranges are rare in practice, unlike the deep,
//! independently authored dependency graphs a general package manager has to
//! solve for.

use std::collections::{HashMap, HashSet};

use crate::error::{Error, Result};
use crate::git;
use crate::jll::project::ProjectToml;
use crate::registry;
use crate::source::{GithubSource, Source};
use crate::version::{CompatSpecifier, Version};

/// How many propagation passes [`resolve`] tries before giving up. Chosen
/// generously: real JLL graphs settle in one or two passes, since they are
/// only a few levels deep, so this budget is about detecting a genuine
/// cycle of ever-tightening constraints rather than a limit resolution is
/// expected to brush up against.
pub const DEFAULT_MAX_ITERATIONS: u32 = 50;

/// One package's outcome from [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    pub name: String,
    /// The chosen version's raw string, for example `7.12.1+0`, exactly as
    /// published. Kept as the original string rather than reformatted from
    /// a parsed [`Version`], since a git tag or lockfile entry must match
    /// what was actually published, build number included.
    pub version: String,
    /// The bare names of this package's own direct JLL dependencies at the
    /// chosen version. Carried into [`crate::lock::LockedPackage`] so the
    /// lock records the same edges the resolver walked.
    pub dependencies: Vec<String>,
}

/// The pieces of a JLL's `Project.toml` the resolver needs at one version:
/// its direct JLL dependencies, each paired with the compat specifier this
/// package declares for it.
#[derive(Debug, Clone)]
pub struct ProjectMeta {
    pub dependencies: Vec<(String, CompatSpecifier)>,
}

/// Where [`resolve`] answers "what versions exist" and "what does this
/// version depend on" from.
///
/// This is the one seam to the network in the whole resolver, kept behind a
/// trait so [`resolve`] itself is tested against an in-memory catalog with
/// no network involved. It is used through a generic parameter rather than
/// a trait object, matching [`crate::source::Source`] and this project's
/// preference for static dispatch.
pub trait Catalog {
    /// Every published version of `package`, each as its raw version
    /// string (used to build a git tag) paired with the parsed [`Version`]
    /// used to compare it. Returns an empty list for a package with no
    /// published releases, which [`resolve`] treats as "not a real
    /// package" (see there for what that means for a root versus a
    /// transitively discovered name).
    fn versions(&self, package: &str) -> Result<Vec<(String, Version)>>;

    /// The dependency metadata for `package` at `version` (its raw version
    /// string, matching one returned by [`Self::versions`]).
    fn metadata(&self, package: &str, version: &str) -> Result<ProjectMeta>;
}

/// The real [`Catalog`], backed by a JLL's GitHub repository: release tags
/// for [`Catalog::versions`], and each tag's `Project.toml` for
/// [`Catalog::metadata`]. This is the same GitHub-per-tag metadata source
/// [`crate::jll::load`] uses elsewhere, just queried for compat bounds
/// instead of the full wrap-generation metadata.
pub struct GithubCatalog;

impl Catalog for GithubCatalog {
    fn versions(&self, package: &str) -> Result<Vec<(String, Version)>> {
        let (owner, repo) = registry::resolve(package);
        // A repository that does not exist at all already reads as "no
        // versions" from `list_tags` itself, the same outcome as a real
        // JLL with zero releases, rather than a hard error here.
        let tags = registry::list_tags(&owner, &repo)?;

        // `list_tags` already learned each tag's commit from the same
        // `git ls-remote` call. `metadata`, right after this, is about to
        // ask `ls_remote_sha` for the exact same (url, tag) pair to key its
        // archive cache, so recording it now turns that into a memo hit
        // instead of a second ~500ms round trip for a fact already known.
        let repo_url = format!("https://github.com/{owner}/{repo}.git");
        for (tag, sha) in &tags {
            git::remember_sha(&repo_url, tag, sha);
        }

        Ok(tags
            .iter()
            .filter_map(|(tag, _sha)| registry::version_from_tag(tag))
            .filter_map(|raw| {
                Version::parse(raw)
                    .ok()
                    .map(|version| (raw.to_string(), version))
            })
            .collect())
    }

    fn metadata(&self, package: &str, version: &str) -> Result<ProjectMeta> {
        let (owner, repo) = registry::resolve(package);
        let git_ref = format!("{package}-v{version}");
        let source = GithubSource::new(owner, repo, git_ref);
        let project_text = source.fetch("Project.toml")?;
        let project = ProjectToml::parse(&project_text)?;

        let dependencies = project
            .jll_dependencies()
            .into_iter()
            .map(|dependency_name| {
                let specifier = project.compat_for_dependency(&dependency_name);
                (dependency_name, specifier)
            })
            .collect();
        Ok(ProjectMeta { dependencies })
    }
}

/// Resolves `required` and its transitive JLL dependencies to one mutually
/// compatible set of versions.
///
/// `pins` overrides the solver's own choice for the given package names.
/// This is how the command layer in [`crate::install`] keeps unrelated
/// packages exactly where a previous lock put them: it pins everything
/// outside the package currently being installed or updated, and leaves
/// only that package's own closure free to move. A pinned version must
/// still satisfy every constraint accumulated against it, so a pin can
/// never produce an inconsistent set, it can only fail resolution outright.
///
/// Fails with [`Error::UnknownJllPackage`] if a name in `required` has no
/// published versions, [`Error::UnknownPin`] if a pinned version is not
/// actually published, [`Error::NoSatisfyingVersion`] if no version of some
/// package satisfies the constraints accumulated against it, and
/// [`Error::ResolutionDidNotConverge`] if the fixed point is not reached
/// within `max_iterations` passes.
///
/// `on_progress` is called with each package's bare name the first time it
/// is visited in a pass, right before the (network-bound) catalog lookups
/// for it, so a caller can report which package resolution is currently
/// waiting on. It may be called more than once for the same name across
/// passes, so it is meant for a live status update, not a per-package tally.
pub fn resolve(
    required: &[String],
    pins: &HashMap<String, String>,
    catalog: &impl Catalog,
    max_iterations: u32,
    mut on_progress: impl FnMut(&str),
) -> Result<HashMap<String, ResolvedPackage>> {
    let required_names: HashSet<String> = required.iter().cloned().collect();

    let mut constraints: HashMap<String, Vec<CompatSpecifier>> = HashMap::new();
    let mut resolved: HashMap<String, ResolvedPackage> = HashMap::new();
    let mut version_cache: HashMap<String, Vec<(String, Version)>> = HashMap::new();
    let mut metadata_cache: HashMap<(String, String), ProjectMeta> = HashMap::new();

    let mut pending: Vec<String> = required.to_vec();

    for _pass in 0..max_iterations {
        let mut changed = false;
        let mut seen_this_pass: HashSet<String> = HashSet::new();

        while let Some(name) = pending.pop() {
            if !seen_this_pass.insert(name.clone()) {
                continue;
            }
            on_progress(&name);

            let available = match version_cache.get(&name) {
                Some(versions) => versions.clone(),
                None => {
                    let versions = catalog.versions(&name)?;
                    version_cache.insert(name.clone(), versions.clone());
                    versions
                }
            };
            if available.is_empty() {
                if required_names.contains(&name) {
                    return Err(Error::UnknownJllPackage { name });
                }
                // Reached only transitively, and the catalog has no
                // published versions for it. Our own
                // `ProjectToml::jll_dependencies` already filters out every
                // non-`_jll` name before it can reach this function, so in
                // practice this branch is never taken today, but it stays
                // as a guard against a future dependency source that is
                // less strict about that filtering.
                continue;
            }

            let candidates: Vec<(String, Version)> = if let Some(pin) = pins.get(&name) {
                let pinned = available
                    .iter()
                    .find(|(raw, _)| raw == pin)
                    .cloned()
                    .ok_or_else(|| Error::UnknownPin {
                        name: name.clone(),
                        pin: pin.clone(),
                    })?;
                vec![pinned]
            } else {
                available
            };

            let no_constraints = Vec::new();
            let package_constraints = constraints.get(&name).unwrap_or(&no_constraints);
            let selected = highest_satisfying(&candidates, package_constraints)
                .ok_or_else(|| Error::NoSatisfyingVersion { name: name.clone() })?;

            let cache_key = (name.clone(), selected.0.clone());
            let metadata = match metadata_cache.get(&cache_key) {
                Some(metadata) => metadata.clone(),
                None => {
                    let metadata = catalog.metadata(&name, &selected.0)?;
                    metadata_cache.insert(cache_key, metadata.clone());
                    metadata
                }
            };

            let previous_version = resolved.get(&name).map(|package| package.version.clone());
            resolved.insert(
                name.clone(),
                ResolvedPackage {
                    name: name.clone(),
                    version: selected.0.clone(),
                    dependencies: metadata
                        .dependencies
                        .iter()
                        .map(|(dependency_name, _)| dependency_name.clone())
                        .collect(),
                },
            );
            if previous_version.as_deref() != Some(selected.0.as_str()) {
                changed = true;
            }

            for (dependency_name, specifier) in metadata.dependencies {
                constraints
                    .entry(dependency_name.clone())
                    .or_default()
                    .push(specifier);
                pending.push(dependency_name);
            }
        }

        if !changed {
            return Ok(resolved);
        }
        pending = resolved.keys().cloned().collect();
    }

    Err(Error::ResolutionDidNotConverge { max_iterations })
}

/// Returns the highest-versioned candidate that satisfies every constraint,
/// or `None` if none does.
fn highest_satisfying(
    candidates: &[(String, Version)],
    constraints: &[CompatSpecifier],
) -> Option<(String, Version)> {
    let mut sorted = candidates.to_vec();
    sorted.sort_by_key(|(_, version)| std::cmp::Reverse(*version));
    sorted.into_iter().find(|(_, version)| {
        constraints
            .iter()
            .all(|specifier| specifier.contains(*version))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `(bare dependency name, compat specifier)` pair, as written in a
    /// test package's declared dependencies.
    type TestDependency = (&'static str, &'static str);

    /// One published version of a test package: its raw version string,
    /// paired with the dependencies it declares at that version.
    type TestVersion = (&'static str, Vec<TestDependency>);

    /// An in-memory [`Catalog`] over a fixed set of packages, each with a
    /// list of published versions, newest last. Used by every test in this
    /// module so resolution is exercised without the network.
    struct TestCatalog {
        packages: HashMap<&'static str, Vec<TestVersion>>,
    }

    impl TestCatalog {
        fn new() -> Self {
            Self {
                packages: HashMap::new(),
            }
        }

        /// Registers one version of `package`, with `dependencies` as
        /// `(bare name, compat specifier)` pairs.
        fn with_version(
            mut self,
            package: &'static str,
            version: &'static str,
            dependencies: Vec<(&'static str, &'static str)>,
        ) -> Self {
            self.packages
                .entry(package)
                .or_default()
                .push((version, dependencies));
            self
        }
    }

    impl Catalog for TestCatalog {
        fn versions(&self, package: &str) -> Result<Vec<(String, Version)>> {
            Ok(self
                .packages
                .get(package)
                .into_iter()
                .flatten()
                .map(|(raw, _)| (raw.to_string(), Version::parse(raw).unwrap()))
                .collect())
        }

        fn metadata(&self, package: &str, version: &str) -> Result<ProjectMeta> {
            let entries = self.packages.get(package).expect("known package");
            let (_, dependencies) = entries
                .iter()
                .find(|(raw, _)| *raw == version)
                .expect("known version");
            Ok(ProjectMeta {
                dependencies: dependencies
                    .iter()
                    .map(|(name, specifier)| (name.to_string(), CompatSpecifier::parse(specifier)))
                    .collect(),
            })
        }
    }

    fn version_of<'a>(resolved: &'a HashMap<String, ResolvedPackage>, name: &str) -> &'a str {
        &resolved
            .get(name)
            .expect("package should be resolved")
            .version
    }

    #[test]
    fn a_satisfiable_graph_resolves_to_the_highest_versions() {
        let catalog = TestCatalog::new()
            .with_version("A", "1.0.0+0", vec![("S", "1.0.0")])
            .with_version("S", "1.0.0+0", vec![])
            .with_version("S", "1.5.0+0", vec![]);

        let required = vec!["A".to_string()];
        let resolved = resolve(
            &required,
            &HashMap::new(),
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |_| {},
        )
        .unwrap();

        assert_eq!(version_of(&resolved, "A"), "1.0.0+0");
        // S's floor from A is "1.0.0", and the highest available clears it.
        assert_eq!(version_of(&resolved, "S"), "1.5.0+0");
    }

    #[test]
    fn an_unsatisfiable_conflict_errors_and_reports_the_package() {
        // A requires S at least 1.0.0, but only ever depends on the
        // "1.0.0+0" release. B requires S at least 2.0.0, a version that
        // does not exist in this catalog at all, so no version of S can
        // satisfy both once B is added.
        let catalog = TestCatalog::new()
            .with_version("A", "1.0.0+0", vec![("S", "1.0.0")])
            .with_version("B", "1.0.0+0", vec![("S", "2.0.0")])
            .with_version("S", "1.0.0+0", vec![]);

        let required = vec!["A".to_string(), "B".to_string()];
        let error = resolve(
            &required,
            &HashMap::new(),
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(error, Error::NoSatisfyingVersion { name } if name == "S"));
    }

    #[test]
    fn a_satisfiable_addition_raises_the_shared_dependency_in_range() {
        // Same shape as the conflict above, but B's floor ("1.2.0") is one
        // S actually publishes and A's compat range still accepts.
        let catalog = TestCatalog::new()
            .with_version("A", "1.0.0+0", vec![("S", "1.0.0")])
            .with_version("B", "1.0.0+0", vec![("S", "1.2.0")])
            .with_version("S", "1.0.0+0", vec![])
            .with_version("S", "1.2.0+0", vec![]);

        let required = vec!["A".to_string(), "B".to_string()];
        let resolved = resolve(
            &required,
            &HashMap::new(),
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |_| {},
        )
        .unwrap();

        assert_eq!(version_of(&resolved, "A"), "1.0.0+0");
        assert_eq!(version_of(&resolved, "B"), "1.0.0+0");
        assert_eq!(version_of(&resolved, "S"), "1.2.0+0");
    }

    #[test]
    fn pinning_one_root_does_not_move_an_unrelated_one() {
        // A and C share nothing. Pinning A to an older version, the way
        // the command layer pins everything outside a refreshed closure,
        // must not disturb C or C's own dependency.
        let catalog = TestCatalog::new()
            .with_version("A", "1.0.0+0", vec![])
            .with_version("A", "2.0.0+0", vec![])
            .with_version("C", "1.0.0+0", vec![("T", "1.0.0")])
            .with_version("T", "1.0.0+0", vec![])
            .with_version("T", "1.5.0+0", vec![]);

        let required = vec!["A".to_string(), "C".to_string()];
        let mut pins = HashMap::new();
        pins.insert("A".to_string(), "1.0.0+0".to_string());

        let resolved = resolve(&required, &pins, &catalog, DEFAULT_MAX_ITERATIONS, |_| {}).unwrap();

        assert_eq!(version_of(&resolved, "A"), "1.0.0+0");
        assert_eq!(version_of(&resolved, "C"), "1.0.0+0");
        assert_eq!(version_of(&resolved, "T"), "1.5.0+0");
    }

    #[test]
    fn update_with_no_pins_equals_install_latest_with_no_pins() {
        let catalog = TestCatalog::new()
            .with_version("A", "1.0.0+0", vec![("S", "1.0.0")])
            .with_version("S", "1.0.0+0", vec![])
            .with_version("S", "2.0.0+0", vec![]);

        let required = vec!["A".to_string()];
        let update_result = resolve(
            &required,
            &HashMap::new(),
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |_| {},
        )
        .unwrap();
        let install_latest_result = resolve(
            &required,
            &HashMap::new(),
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |_| {},
        )
        .unwrap();

        assert_eq!(
            version_of(&update_result, "A"),
            version_of(&install_latest_result, "A")
        );
        assert_eq!(
            version_of(&update_result, "S"),
            version_of(&install_latest_result, "S")
        );
    }

    #[test]
    fn an_unknown_required_package_is_an_error() {
        let catalog = TestCatalog::new();
        let required = vec!["DoesNotExist".to_string()];
        let error = resolve(
            &required,
            &HashMap::new(),
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(error, Error::UnknownJllPackage { name } if name == "DoesNotExist"));
    }

    #[test]
    fn a_pin_that_is_not_published_is_an_error() {
        let catalog = TestCatalog::new().with_version("A", "1.0.0+0", vec![]);
        let required = vec!["A".to_string()];
        let mut pins = HashMap::new();
        pins.insert("A".to_string(), "9.9.9+0".to_string());

        let error =
            resolve(&required, &pins, &catalog, DEFAULT_MAX_ITERATIONS, |_| {}).unwrap_err();
        assert!(
            matches!(error, Error::UnknownPin { name, pin } if name == "A" && pin == "9.9.9+0")
        );
    }
}
