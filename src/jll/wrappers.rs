//! Parsing a JLL's per-platform wrapper script (`src/wrappers/<triplet>.jl`).
//!
//! This file is Julia source, not a data format, so this parser does not
//! embed a Julia interpreter. It only recognises the two macro calls that
//! matter for a Meson dependency: the ones that name a library product and
//! the path it lives at once its tarball is extracted. A real wrapper file
//! looks like this:
//!
//! ```julia
//! JLLWrappers.@declare_library_product(libexample, "libexample.so.3")
//! JLLWrappers.@init_library_product(
//!     libexample,
//!     "lib/libexample.so",
//!     RTLD_LAZY | RTLD_DEEPBIND,
//! )
//! ```
//!
//! `@declare_library_product` gives the soname, `@init_library_product`
//! gives the relative path. Both macro calls exist for every library, so
//! the two are matched independently and then joined by variable name.
//!
//! Only library products are recognised in this first version.
//! `ExecutableProduct` and `FileProduct` are not yet supported, since no
//! Meson dependency needs to expose them the way it needs to expose
//! libraries to link against.

use nom::branch::alt;
use nom::bytes::complete::{tag, take_until};
use nom::character::complete::{char, multispace0};
use nom::combinator::recognize;
use nom::multi::many1;
use nom::sequence::preceded;
use nom::IResult;

/// One library this JLL provides, once its tarball has been extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryProduct {
    /// The Julia variable name, for example `libamd`. Reused as the Meson
    /// variable name for the corresponding `cc.find_library()` result.
    pub variable: String,
    /// The path relative to the extracted tarball, for example
    /// `lib/libamd.so`.
    pub path: String,
    /// The soname declared for this library, for example `libamd.so.3`.
    /// Currently unused by the generator, kept for future use (for example
    /// emitting an explicit `SONAME` check).
    #[allow(dead_code)]
    pub soname: String,
}

/// Scans a wrapper script's source text for library product declarations.
///
/// Products are matched by scanning for `@declare_library_product` and
/// `@init_library_product` calls anywhere in the file and correlating them
/// by variable name, rather than parsing the file as a whole. Julia source
/// around these calls (functions, comments, `using` statements) is ignored.
pub fn parse_library_products(source: &str) -> Vec<LibraryProduct> {
    let sonames = scan_all(source, declare_library_product);
    let paths = scan_all(source, init_library_product);

    sonames
        .into_iter()
        .filter_map(|(variable, soname)| {
            paths
                .iter()
                .find(|(other_variable, _)| *other_variable == variable)
                .map(|(_, path)| LibraryProduct {
                    variable: variable.clone(),
                    path: path.clone(),
                    soname,
                })
        })
        .collect()
}

/// Repeatedly applies `parser` to `input`, skipping forward to the next
/// match each time it fails, until no more matches remain.
fn scan_all<'a, T>(mut input: &'a str, parser: impl Fn(&'a str) -> IResult<&'a str, T>) -> Vec<T> {
    let mut matches = Vec::new();
    while let Ok((rest, value)) = parser(input) {
        matches.push(value);
        input = rest;
    }
    matches
}

/// A Julia identifier: letters, digits, and underscores.
fn identifier(input: &str) -> IResult<&str, &str> {
    recognize(many1(alt((
        nom::character::complete::alphanumeric1,
        nom::bytes::complete::tag("_"),
    ))))(input)
}

/// A double-quoted string literal, without escape handling: the paths and
/// sonames in a wrapper file never contain an escaped quote.
fn string_literal(input: &str) -> IResult<&str, &str> {
    let (input, _) = char('"')(input)?;
    let (input, contents) = take_until("\"")(input)?;
    let (input, _) = char('"')(input)?;
    Ok((input, contents))
}

/// Matches the next `@declare_library_product(name, "soname")` call,
/// skipping any preceding text.
fn declare_library_product(input: &str) -> IResult<&str, (String, String)> {
    let (input, _) = preceded(
        take_until("@declare_library_product"),
        tag("@declare_library_product"),
    )(input)?;
    let (input, _) = char('(')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char(',')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, soname) = string_literal(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char(')')(input)?;
    Ok((input, (name.to_string(), soname.to_string())))
}

/// Matches the next `@init_library_product(name, "path", <flags>)` call,
/// skipping any preceding text. The flags argument is skipped rather than
/// parsed, since only the path is needed.
fn init_library_product(input: &str) -> IResult<&str, (String, String)> {
    let (input, _) = preceded(
        take_until("@init_library_product"),
        tag("@init_library_product"),
    )(input)?;
    let (input, _) = char('(')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char(',')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, path) = string_literal(input)?;
    let (input, _) = take_until(")")(input)?;
    let (input, _) = char(')')(input)?;
    Ok((input, (name.to_string(), path.to_string())))
}

/// One executable this JLL provides, once its tarball has been extracted.
/// A JLL's CLI tool (`highs.exe` alongside `libhighs.dll`, for example),
/// never something a Meson `dependency()` needs to expose, but its path is
/// still useful for telling it apart from the library products a runtime
/// install actually needs (see [`crate::generate::context::TripletOverlayContext`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableProduct {
    /// The Julia variable name, for example `highs`.
    pub variable: String,
    /// The path relative to the extracted tarball, for example
    /// `bin/highs.exe`.
    pub path: String,
}

/// Scans a wrapper script's source text for executable product
/// declarations, the same way [`parse_library_products`] does for
/// libraries: `@declare_executable_product` and `@init_executable_product`
/// calls are matched independently and joined by variable name.
pub fn parse_executable_products(source: &str) -> Vec<ExecutableProduct> {
    let names = scan_all(source, declare_executable_product);
    let paths = scan_all(source, init_executable_product);

    names
        .into_iter()
        .filter_map(|variable| {
            paths
                .iter()
                .find(|(other_variable, _)| *other_variable == variable)
                .map(|(_, path)| ExecutableProduct {
                    variable: variable.clone(),
                    path: path.clone(),
                })
        })
        .collect()
}

/// Matches the next `@declare_executable_product(name)` call, skipping any
/// preceding text. Unlike a library product, there is no soname argument.
fn declare_executable_product(input: &str) -> IResult<&str, String> {
    let (input, _) = preceded(
        take_until("@declare_executable_product"),
        tag("@declare_executable_product"),
    )(input)?;
    let (input, _) = char('(')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char(')')(input)?;
    Ok((input, name.to_string()))
}

/// Matches the next `@init_executable_product(name, "path")` call, skipping
/// any preceding text.
fn init_executable_product(input: &str) -> IResult<&str, (String, String)> {
    let (input, _) = preceded(
        take_until("@init_executable_product"),
        tag("@init_executable_product"),
    )(input)?;
    let (input, _) = char('(')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char(',')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, path) = string_literal(input)?;
    let (input, _) = take_until(")")(input)?;
    let (input, _) = char(')')(input)?;
    Ok((input, (name.to_string(), path.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
        using OtherThing_jll

        export libexample, libother

        const libexample_path = ""

        JLLWrappers.@declare_library_product(libexample, "libexample.so.3")
        JLLWrappers.@declare_library_product(libother, "libother.so.5")

        function __init__()
            JLLWrappers.@init_library_product(
                libexample,
                "lib/libexample.so",
                RTLD_LAZY | RTLD_DEEPBIND,
            )
            JLLWrappers.@init_library_product(
                libother,
                "lib/libother.so",
                RTLD_LAZY | RTLD_DEEPBIND,
            )
        end
    "#;

    #[test]
    fn parses_every_library_product() {
        let products = parse_library_products(EXAMPLE);
        assert_eq!(
            products,
            vec![
                LibraryProduct {
                    variable: "libexample".to_string(),
                    path: "lib/libexample.so".to_string(),
                    soname: "libexample.so.3".to_string(),
                },
                LibraryProduct {
                    variable: "libother".to_string(),
                    path: "lib/libother.so".to_string(),
                    soname: "libother.so.5".to_string(),
                },
            ]
        );
    }

    #[test]
    fn empty_source_yields_no_products() {
        assert_eq!(parse_library_products(""), Vec::new());
    }

    const EXECUTABLE_EXAMPLE: &str = r#"
        JLLWrappers.@declare_library_product(libhighs, "libhighs.dll")
        JLLWrappers.@declare_executable_product(highs)
        function __init__()
            JLLWrappers.@init_library_product(
                libhighs,
                "bin\\libhighs.dll",
                RTLD_LAZY | RTLD_DEEPBIND,
            )
            JLLWrappers.@init_executable_product(
                highs,
                "bin\\highs.exe",
            )
        end
    "#;

    #[test]
    fn parses_an_executable_product_alongside_a_library_product() {
        let products = parse_executable_products(EXECUTABLE_EXAMPLE);
        assert_eq!(
            products,
            vec![ExecutableProduct {
                variable: "highs".to_string(),
                path: r"bin\\highs.exe".to_string(),
            }]
        );
    }

    #[test]
    fn empty_source_yields_no_executable_products() {
        assert_eq!(parse_executable_products(""), Vec::new());
    }
}
