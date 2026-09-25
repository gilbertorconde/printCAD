//! Packages from GitHub releases: a repository's latest release (or a
//! tagged one) carries the package as a `.pcbench` asset. Installing one
//! records where it came from (`source.json` in the package folder), which
//! is what checking for and taking updates reads.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::package::{self, ARCHIVE_EXTENSION, Package};

/// Where an installed package came from.
pub const SOURCE: &str = "source.json";

/// The largest package archive taken from the network.
const ARCHIVE_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// `owner/repo` on GitHub.
    pub repo: String,
    /// The release tag it was installed from.
    pub tag: String,
    /// The asset's file name.
    pub asset: String,
}

impl Source {
    pub fn url(&self) -> String {
        format!("https://github.com/{}", self.repo)
    }
}

/// A release's package asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub asset: String,
    pub download: String,
    /// `sha256:<hex>`, when GitHub gives one.
    pub digest: Option<String>,
}

/// What reaches the network: GitHub's API and downloads. Tests stand in
/// their own.
pub trait Fetch {
    fn json(&self, url: &str) -> Result<Value, String>;
    fn bytes(&self, url: &str, limit: u64) -> Result<Vec<u8>, String>;
}

/// HTTPS, through the system's certificate roots.
pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .user_agent(concat!("printcad/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent }
    }
}

impl Fetch for Http {
    fn json(&self, url: &str) -> Result<Value, String> {
        let mut response = self
            .agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| http_error(url, e))?;
        response
            .body_mut()
            .read_json()
            .map_err(|e| format!("{url} answered what does not read: {e}"))
    }

    fn bytes(&self, url: &str, limit: u64) -> Result<Vec<u8>, String> {
        let mut response = self
            .agent
            .get(url)
            .header("Accept", "application/octet-stream")
            .call()
            .map_err(|e| http_error(url, e))?;
        response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| format!("cannot download {url}: {e}"))
    }
}

fn http_error(url: &str, error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(404) => format!("{url} was not found"),
        ureq::Error::StatusCode(403) => {
            "GitHub refused the request; its limit for requests without an account may be \
             reached, try again in an hour"
                .into()
        }
        other => format!("cannot reach {url}: {other}"),
    }
}

/// `owner/repo` and the tag asked for, from what the user typed: a
/// repository's address, a release's (`…/releases/tag/v1.2`), or
/// `owner/repo`.
pub fn parse_repo(text: &str) -> Result<(String, Option<String>), String> {
    let bad = || {
        format!(
            "`{}` is not a GitHub repository; write its address, such as \
             https://github.com/owner/repo",
            text.trim()
        )
    };
    let rest = text
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("github.com/")
        .trim_end_matches('/');
    let mut parts = rest.split('/');
    let owner = parts.next().filter(|s| !s.is_empty()).ok_or_else(bad)?;
    let repo = parts
        .next()
        .filter(|s| !s.is_empty())
        .map(|r| r.trim_end_matches(".git"))
        .ok_or_else(bad)?;
    let plain = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if !plain(owner) || !plain(repo) || rest.contains("..") {
        return Err(bad());
    }
    let tag = match (parts.next(), parts.next(), parts.next()) {
        (None, _, _) => None,
        (Some("releases"), None, _) | (Some("releases"), Some("latest"), None) => None,
        (Some("releases"), Some("tag"), Some(tag)) if !tag.is_empty() => Some(tag.to_string()),
        _ => return Err(bad()),
    };
    Ok((format!("{owner}/{repo}"), tag))
}

/// The package asset of `repo`'s release `tag`, or of its latest release.
pub fn release(fetch: &dyn Fetch, repo: &str, tag: Option<&str>) -> Result<Release, String> {
    let url = match tag {
        Some(tag) => format!("https://api.github.com/repos/{repo}/releases/tags/{tag}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let json = fetch.json(&url).map_err(|e| {
        if tag.is_none() && e.ends_with("was not found") {
            format!("{repo} has no published release")
        } else {
            e
        }
    })?;
    let tag = json
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or("the release has no tag")?
        .to_string();
    let suffix = format!(".{ARCHIVE_EXTENSION}");
    let asset = json
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| {
            a.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.ends_with(&suffix))
        })
        .ok_or_else(|| format!("release {tag} of {repo} has no {suffix} file"))?;
    Ok(Release {
        tag,
        asset: asset["name"].as_str().unwrap_or_default().to_string(),
        download: asset
            .get("browser_download_url")
            .and_then(Value::as_str)
            .ok_or("the release's package has no download address")?
            .to_string(),
        digest: asset
            .get("digest")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Download `release`'s archive, checked against its digest when GitHub
/// gave one.
fn download(fetch: &dyn Fetch, release: &Release) -> Result<Vec<u8>, String> {
    let bytes = fetch.bytes(&release.download, ARCHIVE_LIMIT)?;
    if let Some(digest) = &release.digest
        && let Some(want) = digest.strip_prefix("sha256:")
    {
        let got: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !got.eq_ignore_ascii_case(want) {
            return Err(format!(
                "{} did not arrive whole (its checksum differs); nothing was installed",
                release.asset
            ));
        }
    }
    Ok(bytes)
}

/// Install the package that `text` (a repository or release address)
/// publishes, under `root`.
pub fn install_from_github(fetch: &dyn Fetch, text: &str, root: &Path) -> Result<Package, String> {
    let (repo, tag) = parse_repo(text)?;
    let release = release(fetch, &repo, tag.as_deref())?;
    let bytes = download(fetch, &release)?;
    let package = package::install_bytes(&bytes, root, None)?;
    write_source(
        &package,
        &Source {
            repo,
            tag: release.tag,
            asset: release.asset,
        },
    )?;
    Ok(package)
}

fn write_source(package: &Package, source: &Source) -> Result<(), String> {
    let json = serde_json::to_string_pretty(source).map_err(|e| e.to_string())?;
    std::fs::write(package.dir.join(SOURCE), json)
        .map_err(|e| format!("cannot record where {} came from: {e}", package.manifest.id))
}

/// Where `package` came from, when it was installed from GitHub.
pub fn source_of(package: &Package) -> Option<Source> {
    let text = std::fs::read_to_string(package.dir.join(SOURCE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The release newer than what `package` was installed from, if there is
/// one. A package installed from a file has none to check.
pub fn check(fetch: &dyn Fetch, package: &Package) -> Result<Option<Release>, String> {
    let Some(source) = source_of(package) else {
        return Ok(None);
    };
    let latest = release(fetch, &source.repo, None)?;
    Ok(newer(&latest.tag, &source.tag, &package.manifest.version).then_some(latest))
}

/// Whether release `latest` is newer than the one installed (`tag`, with
/// the manifest's `version`): never when it is the installed tag, else by
/// version number where both read as one (the tag's, or the manifest's
/// when the tag has none), else by tag.
pub fn newer(latest: &str, tag: &str, version: &str) -> bool {
    if latest == tag {
        return false;
    }
    match (numbers(latest), numbers(tag).or_else(|| numbers(version))) {
        (Some(l), Some(i)) => l > i,
        _ => true,
    }
}

/// `v1.2.3`, `1.2`, `release-1.2.3-beta` as its numbers.
fn numbers(text: &str) -> Option<Vec<u64>> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let core: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts: Option<Vec<u64>> = core
        .split('.')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().ok())
        .collect();
    parts.filter(|p| !p.is_empty())
}

/// Replace `package` with `release`, keeping its data and its source. The
/// release must hold the same package: a repository that starts shipping
/// another is refused.
pub fn update(fetch: &dyn Fetch, package: &Package, release: &Release) -> Result<Package, String> {
    let source = source_of(package).ok_or("the package was not installed from GitHub")?;
    let bytes = download(fetch, release)?;
    let root = package
        .dir
        .parent()
        .ok_or("the package has no folder above it")?;
    let updated = package::install_bytes(&bytes, root, Some(&package.manifest.id))?;
    write_source(
        &updated,
        &Source {
            repo: source.repo,
            tag: release.tag.clone(),
            asset: release.asset.clone(),
        },
    )?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_reads_from_any_way_it_is_written() {
        for text in [
            "https://github.com/acme/cam",
            "github.com/acme/cam/",
            "acme/cam",
            "https://github.com/acme/cam.git",
            "https://github.com/acme/cam/releases/latest",
        ] {
            assert_eq!(parse_repo(text), Ok(("acme/cam".into(), None)), "{text}");
        }
        assert_eq!(
            parse_repo("https://github.com/acme/cam/releases/tag/v0.3.0"),
            Ok(("acme/cam".into(), Some("v0.3.0".into())))
        );
        for text in [
            "acme",
            "https://github.com/acme/cam/issues/3",
            "a b/c",
            "acme/../x",
        ] {
            assert!(parse_repo(text).is_err(), "{text}");
        }
    }

    /// Reaches GitHub: `cargo test -p wb_wasm -- --ignored reaches_github`.
    #[test]
    #[ignore = "reaches the network"]
    fn reaches_github_and_reads_a_real_release() {
        let found = release(&Http::default(), "bytecodealliance/wasmtime", None);
        let error = found.expect_err("wasmtime's releases carry no package");
        assert!(error.contains("has no .pcbench file"), "{error}");
        let missing = release(
            &Http::default(),
            "gilbertorconde/no-such-repository-here",
            None,
        )
        .unwrap_err();
        assert!(missing.contains("no published release"), "{missing}");
    }

    #[test]
    fn a_release_is_newer_by_its_number_or_else_its_tag() {
        assert!(newer("v0.4.0", "v0.3.0", "0.3.0"));
        assert!(newer("v0.10.0", "v0.9.2", "0.9.2"));
        assert!(!newer("v0.3.0", "v0.3.0", "0.3.0"));
        assert!(!newer("v0.2.9", "v0.3.0", "0.3.0"), "never back");
        assert!(newer("nightly-b", "nightly-a", "dev"));
        assert!(!newer("nightly-a", "nightly-a", "dev"));
        assert!(
            !newer("v0.4.0", "v0.4.0", "0.1.0"),
            "the tag installed, whatever the manifest says"
        );
    }
}
