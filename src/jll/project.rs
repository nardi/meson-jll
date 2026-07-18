//! Parsing a JLL's `Project.toml`.
//!
//! This file carries the package name, its version, and its dependencies.
//! Only the dependencies whose name ends in `_jll` matter to the generator,
//! since those are the ones that need their own wrap set. Every other
//! dependency (`JLLWrappers`, `Libdl`, `Artifacts`, and so on) is Julia
//! standard library plumbing with no binary of its own, and is ignored.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Error, Result};

/// The parsed contents of a JLL's `Project.toml`.
#[derive(Debug, Deserialize)]
pub struct ProjectToml {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub deps: HashMap<String, String>,
}

impl ProjectToml {
    /// Parses `Project.toml` file contents.
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|source| Error::ParseProjectToml { source })
    }

    /// The package name with its `_jll` suffix removed, for example
    /// `ExampleThing` from `ExampleThing_jll`. This is the name used
    /// everywhere in the generated wraps: as the public dependency name and
    /// as the file name prefix.
    pub fn bare_name(&self) -> &str {
        self.name.strip_suffix("_jll").unwrap_or(&self.name)
    }

    /// The bare names of the other JLL packages this one depends on, for
    /// example `OtherThing` from `OtherThing_jll`.
    pub fn jll_dependencies(&self) -> Vec<String> {
        self.deps
            .keys()
            .filter_map(|name| name.strip_suffix("_jll"))
            .map(String::from)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
        name = "ExampleThing_jll"
        uuid = "00000000-0000-0000-0000-000000000000"
        version = "1.2.3+0"

        [deps]
        JLLWrappers = "692b3bcd-3c85-4b1f-b108-f13ce0eb3210"
        OtherThing_jll = "11111111-1111-1111-1111-111111111111"
        Libdl = "8f399da3-3557-5675-b5ff-fb832c97cbdb"
        Artifacts = "56f22d72-fd6d-98f1-02f0-08ddc0907c33"
    "#;

    #[test]
    fn parses_name_and_version() {
        let project = ProjectToml::parse(EXAMPLE).unwrap();
        assert_eq!(project.bare_name(), "ExampleThing");
        assert_eq!(project.version, "1.2.3+0");
    }

    #[test]
    fn finds_only_the_jll_dependency() {
        let project = ProjectToml::parse(EXAMPLE).unwrap();
        assert_eq!(project.jll_dependencies(), vec!["OtherThing"]);
    }
}
