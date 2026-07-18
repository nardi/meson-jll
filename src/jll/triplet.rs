//! The platform triplet model.
//!
//! A JLL's `Artifacts.toml` describes each supported platform with a handful
//! of separate fields (`arch`, `os`, `libc`, and so on) rather than a single
//! triplet string. [`Triplet`] gathers those fields into one value and knows
//! how to translate them to the two things the generator needs: the values
//! Meson's `host_machine` exposes, and a stable identifier used to name the
//! files this tool generates.
//!
//! The identifier this module produces is `meson-jll`'s own naming scheme.
//! It does not need to match Julia's internal triplet strings exactly,
//! because it is only used to name files this tool writes, never sent back
//! to Julia. It happens to be close to Julia's own convention, which is a
//! side effect of following the same arch-os-libc-abi ordering, not a
//! requirement.

/// A CPU architecture, as named in an `Artifacts.toml` `arch` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    I686,
    Aarch64,
    Armv6l,
    Armv7l,
    Powerpc64le,
    Riscv64,
}

impl Arch {
    /// Parses the `arch` field of an `Artifacts.toml` platform entry.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "x86_64" => Self::X86_64,
            "i686" => Self::I686,
            "aarch64" => Self::Aarch64,
            "armv6l" => Self::Armv6l,
            "armv7l" => Self::Armv7l,
            "powerpc64le" => Self::Powerpc64le,
            "riscv64" => Self::Riscv64,
            _ => return None,
        })
    }

    /// The word used for this architecture in generated file names.
    pub fn identifier(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::I686 => "i686",
            Self::Aarch64 => "aarch64",
            Self::Armv6l => "armv6l",
            Self::Armv7l => "armv7l",
            Self::Powerpc64le => "powerpc64le",
            Self::Riscv64 => "riscv64",
        }
    }

    /// The value Meson's `host_machine.cpu_family()` returns on this
    /// architecture.
    pub fn meson_cpu_family(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::I686 => "x86",
            Self::Aarch64 => "aarch64",
            Self::Armv6l | Self::Armv7l => "arm",
            Self::Powerpc64le => "ppc64",
            Self::Riscv64 => "riscv64",
        }
    }

    /// The value Meson's `host_machine.cpu()` reports for this
    /// architecture, for the cases where `cpu_family()` alone cannot tell
    /// two architectures apart.
    ///
    /// 32-bit ARM is the one case that matters here: both `armv6l` and
    /// `armv7l` report the same `arm` family from `cpu_family()`, and can
    /// only be told apart through the more specific `cpu()` value. Returns
    /// `None` when `cpu_family()` is already unambiguous, so the selector
    /// only adds the extra check where it is actually needed.
    pub fn meson_cpu_disambiguator(self) -> Option<&'static str> {
        match self {
            Self::Armv6l => Some("armv6l"),
            Self::Armv7l => Some("armv7l"),
            _ => None,
        }
    }

    /// The value MSVC's `lib.exe /machine:` flag expects for this
    /// architecture, needed only to regenerate an MSVC-compatible import
    /// library from a MinGW-built Windows DLL (see
    /// `crate::generate::write_triplet_overlay`). `None` for an
    /// architecture Windows JLL builds do not actually target, since no
    /// JLL ever needs this for one.
    pub fn msvc_machine(self) -> Option<&'static str> {
        match self {
            Self::X86_64 => Some("X64"),
            Self::I686 => Some("X86"),
            Self::Aarch64 => Some("ARM64"),
            Self::Armv6l | Self::Armv7l | Self::Powerpc64le | Self::Riscv64 => None,
        }
    }
}

/// An operating system, as named in an `Artifacts.toml` `os` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    FreeBsd,
}

impl Os {
    /// Parses the `os` field of an `Artifacts.toml` platform entry.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "linux" => Self::Linux,
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            "freebsd" => Self::FreeBsd,
            _ => return None,
        })
    }

    /// The word used for this operating system in generated file names.
    pub fn identifier(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "darwin",
            Self::Windows => "windows",
            Self::FreeBsd => "freebsd",
        }
    }

    /// The value Meson's `host_machine.system()` returns on this operating
    /// system.
    pub fn meson_system(self) -> &'static str {
        // These happen to already match `identifier()` today, but the two
        // are conceptually different (one is our file naming, the other is
        // Meson's vocabulary), so they are kept as separate methods.
        self.identifier()
    }

    /// The word Julia's own BinaryBuilder triplet convention uses for this
    /// operating system, needed to find a JLL's `src/wrappers/<triplet>.jl`
    /// (see [`Triplet::julia_wrapper_identifier`]). Matches `identifier()`
    /// for Linux, but not for the other three: Julia's own convention is
    /// `apple-darwin`, `unknown-freebsd`, and `w64-mingw32` where this
    /// tool's own file naming just says `darwin`, `freebsd`, and `windows`.
    fn julia_identifier(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "apple-darwin",
            Self::Windows => "w64-mingw32",
            Self::FreeBsd => "unknown-freebsd",
        }
    }
}

/// The C standard library on a Linux platform. Not meaningful anywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Libc {
    Glibc,
    Musl,
}

impl Libc {
    /// Parses the `libc` field of an `Artifacts.toml` platform entry.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "glibc" => Self::Glibc,
            "musl" => Self::Musl,
            _ => return None,
        })
    }

    /// The word used for this libc in generated file names, matching the
    /// suffix Julia itself uses (`gnu` or `musl`).
    pub fn identifier(self) -> &'static str {
        match self {
            Self::Glibc => "gnu",
            Self::Musl => "musl",
        }
    }
}

/// The calling convention on a 32-bit ARM platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallAbi {
    HardFloat,
    SoftFloat,
}

impl CallAbi {
    /// Parses the `call_abi` field of an `Artifacts.toml` platform entry.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "eabihf" => Self::HardFloat,
            "eabi" => Self::SoftFloat,
            _ => return None,
        })
    }

    /// The word appended directly after the libc identifier in generated
    /// file names, matching Julia's own `gnueabihf` / `musleabihf` suffixes.
    pub fn identifier(self) -> &'static str {
        match self {
            Self::HardFloat => "eabihf",
            Self::SoftFloat => "eabi",
        }
    }
}

/// A fully described platform: architecture, operating system, and the
/// optional selectors that only apply on some of them.
#[derive(Debug, Clone)]
pub struct Triplet {
    pub arch: Arch,
    pub os: Os,
    pub libc: Option<Libc>,
    pub call_abi: Option<CallAbi>,
    /// The raw `cxxstring_abi` tag from `Artifacts.toml`, when this JLL
    /// splits a platform by C++ standard library ABI (for example `cxx11`).
    pub cxxstring_abi: Option<String>,
    /// The raw `libgfortran_version` tag from `Artifacts.toml`, when this
    /// JLL splits a platform by Fortran runtime version (for example `5`).
    pub libgfortran_version: Option<String>,
}

impl Triplet {
    /// The identifier used to name this triplet's generated wrap file and
    /// subproject directory, for example `x86_64-linux-gnu` or
    /// `x86_64-linux-gnu-cxx11`.
    pub fn identifier(&self) -> String {
        let mut identifier = format!("{}-{}", self.arch.identifier(), self.os.identifier());
        if let Some(libc) = self.libc {
            identifier.push('-');
            identifier.push_str(libc.identifier());
        }
        if let Some(call_abi) = self.call_abi {
            identifier.push_str(call_abi.identifier());
        }
        if let Some(abi) = &self.cxxstring_abi {
            identifier.push_str("-cxx");
            identifier.push_str(abi.trim_start_matches("cxx"));
        }
        if let Some(version) = &self.libgfortran_version {
            identifier.push_str("-libgfortran");
            identifier.push_str(version);
        }
        identifier
    }

    /// The triplet string Julia's own BinaryBuilder convention names this
    /// platform's `src/wrappers/<this>.jl` with, which must be matched
    /// exactly to find that file: this tool does not otherwise know that
    /// file's name, only its own naming scheme from [`Self::identifier`].
    ///
    /// The two agree on everything except the operating system word (see
    /// [`Os::julia_identifier`]) and, when this JLL splits a platform by
    /// Fortran runtime version, on how much of it is used: this tool's own
    /// naming keeps the full `Artifacts.toml` value (for example `5.0.0`),
    /// but Julia's wrapper filenames (and, as it happens, its release
    /// tarball names) use only the major version (`libgfortran5`).
    pub fn julia_wrapper_identifier(&self) -> String {
        let mut identifier = format!("{}-{}", self.arch.identifier(), self.os.julia_identifier());
        if let Some(libc) = self.libc {
            identifier.push('-');
            identifier.push_str(libc.identifier());
        }
        if let Some(call_abi) = self.call_abi {
            identifier.push_str(call_abi.identifier());
        }
        if let Some(abi) = &self.cxxstring_abi {
            identifier.push_str("-cxx");
            identifier.push_str(abi.trim_start_matches("cxx"));
        }
        if let Some(version) = &self.libgfortran_version {
            let major_version = version.split('.').next().unwrap_or(version);
            identifier.push_str("-libgfortran");
            identifier.push_str(major_version);
        }
        identifier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_for_a_plain_linux_platform() {
        let triplet = Triplet {
            arch: Arch::X86_64,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: None,
            cxxstring_abi: None,
            libgfortran_version: None,
        };
        assert_eq!(triplet.identifier(), "x86_64-linux-gnu");
    }

    #[test]
    fn identifier_for_an_arm_hard_float_platform() {
        let triplet = Triplet {
            arch: Arch::Armv7l,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: Some(CallAbi::HardFloat),
            cxxstring_abi: None,
            libgfortran_version: None,
        };
        assert_eq!(triplet.identifier(), "armv7l-linux-gnueabihf");
    }

    #[test]
    fn identifier_with_abi_variants() {
        let triplet = Triplet {
            arch: Arch::X86_64,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: None,
            cxxstring_abi: Some("cxx11".to_string()),
            libgfortran_version: Some("5".to_string()),
        };
        assert_eq!(triplet.identifier(), "x86_64-linux-gnu-cxx11-libgfortran5");
    }

    #[test]
    fn identifier_for_macos_has_no_libc() {
        let triplet = Triplet {
            arch: Arch::Aarch64,
            os: Os::MacOs,
            libc: None,
            call_abi: None,
            cxxstring_abi: None,
            libgfortran_version: None,
        };
        assert_eq!(triplet.identifier(), "aarch64-darwin");
    }

    /// Regression test: `identifier()` (this tool's own file naming) and
    /// `julia_wrapper_identifier()` (Julia's own convention, needed to find
    /// `src/wrappers/<this>.jl`) used to be the same method, so a wrapper
    /// script fetch silently found nothing on macOS, FreeBSD, and Windows,
    /// where the two conventions actually differ, and every library on
    /// those platforms went unlinked without any error at all.
    #[test]
    fn julia_wrapper_identifier_differs_from_own_identifier_on_macos() {
        let triplet = Triplet {
            arch: Arch::X86_64,
            os: Os::MacOs,
            libc: None,
            call_abi: None,
            cxxstring_abi: None,
            libgfortran_version: None,
        };
        assert_eq!(triplet.identifier(), "x86_64-darwin");
        assert_eq!(triplet.julia_wrapper_identifier(), "x86_64-apple-darwin");
    }

    #[test]
    fn julia_wrapper_identifier_differs_from_own_identifier_on_freebsd() {
        let triplet = Triplet {
            arch: Arch::X86_64,
            os: Os::FreeBsd,
            libc: None,
            call_abi: None,
            cxxstring_abi: None,
            libgfortran_version: None,
        };
        assert_eq!(triplet.identifier(), "x86_64-freebsd");
        assert_eq!(triplet.julia_wrapper_identifier(), "x86_64-unknown-freebsd");
    }

    #[test]
    fn julia_wrapper_identifier_differs_from_own_identifier_on_windows() {
        let triplet = Triplet {
            arch: Arch::X86_64,
            os: Os::Windows,
            libc: None,
            call_abi: None,
            cxxstring_abi: None,
            libgfortran_version: None,
        };
        assert_eq!(triplet.identifier(), "x86_64-windows");
        assert_eq!(triplet.julia_wrapper_identifier(), "x86_64-w64-mingw32");
    }

    #[test]
    fn julia_wrapper_identifier_uses_only_the_major_libgfortran_version() {
        let triplet = Triplet {
            arch: Arch::X86_64,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: None,
            cxxstring_abi: None,
            libgfortran_version: Some("5.0.0".to_string()),
        };
        assert_eq!(triplet.identifier(), "x86_64-linux-gnu-libgfortran5.0.0");
        assert_eq!(
            triplet.julia_wrapper_identifier(),
            "x86_64-linux-gnu-libgfortran5"
        );
    }

    #[test]
    fn julia_wrapper_identifier_matches_own_identifier_on_linux() {
        let triplet = Triplet {
            arch: Arch::Armv7l,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: Some(CallAbi::HardFloat),
            cxxstring_abi: Some("cxx11".to_string()),
            libgfortran_version: None,
        };
        assert_eq!(triplet.identifier(), triplet.julia_wrapper_identifier());
    }
}
