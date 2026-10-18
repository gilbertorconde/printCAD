//! The workbench store: an index of listed packages, each in its author's
//! own GitHub repository, published by the registry (a repository of one
//! reviewed entry per package, whose CI reads each package's latest
//! release into the index). The index carries what installing needs (the
//! download and its sha256), so browsing and installing ask GitHub's API
//! for nothing; a package installed from it records its repository as one
//! installed from a GitHub address does, and updates the same way.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::package::{self, Package};
use crate::remote::{self, Fetch, Release, Source};

/// The index format this app reads.
pub const SCHEMA: u32 = 1;

/// The registry's index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    /// The store's name, as its registry calls itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// When the registry built it.
    #[serde(default)]
    pub generated: String,
    pub packages: Vec<Listing>,
}

/// One listed package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `owner/repo` on GitHub.
    pub repository: String,
    #[serde(default)]
    pub maintainers: Vec<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Taken off the list, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<String>,
    /// Its latest release; `None` while the registry could not read one.
    #[serde(default)]
    pub release: Option<Listed>,
    /// Why there is no release to install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// A listed package's latest release, as the registry read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    pub tag: String,
    #[serde(default)]
    pub version: String,
    /// The workbench contract it was built against.
    pub api: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    /// Its page on GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    pub asset: String,
    pub url: String,
    #[serde(default)]
    pub size: u64,
    pub sha256: String,
    /// What it asks to reach beyond its own folder.
    #[serde(default)]
    pub capabilities: crate::Capabilities,
}

impl Listing {
    /// What there is to install: its release, when it is listed, has one,
    /// and runs in this app.
    pub fn installable(&self) -> Result<&Listed, String> {
        if let Some(why) = &self.removed {
            return Err(format!("{} was taken off the list: {why}", self.name));
        }
        let release = self.release.as_ref().ok_or_else(|| {
            self.problem
                .clone()
                .unwrap_or_else(|| format!("{} has no release to install", self.name))
        })?;
        if !bench_api::api_compatible(&release.api) {
            return Err(format!(
                "{} {} targets {}; this printCAD speaks printcad:workbench@{}",
                self.name,
                release.version,
                release.api,
                bench_api::API_VERSION
            ));
        }
        Ok(release)
    }
}

/// The index at `url`. One in a format this app does not read is refused
/// with a word on why.
pub fn fetch_index(fetch: &dyn Fetch, url: &str) -> Result<Index, String> {
    read_index(fetch.json(url)?)
}

/// An index from its JSON.
pub fn read_index(json: serde_json::Value) -> Result<Index, String> {
    let schema = json.get("schema").and_then(serde_json::Value::as_u64);
    if schema != Some(u64::from(SCHEMA)) {
        return Err(match schema {
            Some(n) if n > u64::from(SCHEMA) => {
                "the workbench list is in a newer format: update printCAD to browse it".into()
            }
            _ => "the workbench list does not read as one".into(),
        });
    }
    serde_json::from_value(json).map_err(|e| format!("the workbench list does not read: {e}"))
}

/// An index from its text, as the app keeps one between runs.
pub fn read_index_text(text: &str) -> Result<Index, String> {
    read_index(
        serde_json::from_str(text).map_err(|e| format!("the workbench list does not read: {e}"))?,
    )
}

/// Install `listing`'s release under `root`: downloaded, checked against
/// the index's sha256, and refused unless it holds the package listed. It
/// remembers its repository, so its updates come from there.
pub fn install_listed(
    fetch: &dyn Fetch,
    listing: &Listing,
    root: &Path,
) -> Result<Package, String> {
    let listed = listing.installable()?;
    let release = Release {
        tag: listed.tag.clone(),
        asset: listed.asset.clone(),
        download: listed.url.clone(),
        digest: Some(format!("sha256:{}", listed.sha256)),
    };
    let bytes = remote::download(fetch, &release)?;
    let installed = package::install_bytes(&bytes, root, Some(&listing.id))?;
    remote::write_source(
        &installed,
        &Source {
            repo: listing.repository.clone(),
            tag: listed.tag.clone(),
            asset: listed.asset.clone(),
        },
    )?;
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use sha2::Digest;

    fn index(release: Value) -> Value {
        json!({
            "schema": 1,
            "api": "0.1",
            "generated": "2026-10-01T09:00:00Z",
            "packages": [{
                "id": "acme.cam",
                "name": "CAM",
                "description": "Toolpaths",
                "repository": "acme/printcad-cam",
                "maintainers": ["acme-dev"],
                "license": "MIT",
                "categories": ["printing"],
                "release": release,
            }],
        })
    }

    fn release(sha256: &str, api: &str) -> Value {
        json!({
            "tag": "v0.3.0",
            "version": "0.3.0",
            "api": api,
            "asset": "acme.cam-0.3.0.pcbench",
            "url": "https://example.invalid/acme.cam-0.3.0.pcbench",
            "size": 10,
            "sha256": sha256,
            "capabilities": {"save_dialog": true, "helper": false, "network": true},
        })
    }

    /// A package archive holding `id`.
    fn archive(id: &str) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);
            for (name, data) in [
                (
                    "bench.toml",
                    format!(
                        "id = \"{id}\"\nname = \"CAM\"\nversion = \"0.3.0\"\napi = \"printcad:workbench@0.1\"\n"
                    )
                    .into_bytes(),
                ),
                ("bench.wasm", b"\0asm".to_vec()),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, name, data.as_slice()).unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap();
        }
        out
    }

    /// Serves one archive for every download.
    struct Serves(Vec<u8>);

    impl Fetch for Serves {
        fn json(&self, _url: &str) -> Result<Value, String> {
            Err("no API here".into())
        }
        fn bytes(&self, _url: &str, _limit: u64) -> Result<Vec<u8>, String> {
            Ok(self.0.clone())
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        sha2::Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn an_index_reads_with_its_releases_and_what_they_ask() {
        let read = read_index(index(release("ab", "printcad:workbench@0.1"))).unwrap();
        let listing = &read.packages[0];
        assert_eq!(listing.repository, "acme/printcad-cam");
        let listed = listing.installable().unwrap();
        assert!(listed.capabilities.network && !listed.capabilities.helper);
    }

    #[test]
    fn an_index_in_another_format_is_refused_with_a_reason() {
        let mut newer = index(Value::Null);
        newer["schema"] = json!(2);
        assert!(read_index(newer).unwrap_err().contains("update printCAD"));
        assert!(read_index(json!({"packages": []})).is_err());
    }

    #[test]
    fn a_removed_unreleased_or_incompatible_listing_has_nothing_to_install() {
        let read = |release: Value, extra: Value| {
            let mut i = index(release);
            for (k, v) in extra.as_object().unwrap() {
                i["packages"][0][k] = v.clone();
            }
            read_index(i).unwrap().packages.remove(0)
        };
        let gone = read(
            release("ab", "printcad:workbench@0.1"),
            json!({"removed": "abandoned"}),
        );
        assert!(gone.installable().unwrap_err().contains("abandoned"));
        let none = read(Value::Null, json!({"problem": "no published release"}));
        assert!(
            none.installable()
                .unwrap_err()
                .contains("no published release")
        );
        let future = read(release("ab", "printcad:workbench@0.9"), json!({}));
        assert!(future.installable().unwrap_err().contains("targets"));
    }

    #[test]
    fn a_listed_package_installs_checked_and_remembers_its_repository() {
        let root = std::env::temp_dir().join(format!("printcad-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = archive("acme.cam");
        let listing = read_index(index(release(&sha256(&bytes), "printcad:workbench@0.1")))
            .unwrap()
            .packages
            .remove(0);
        let installed = install_listed(&Serves(bytes.clone()), &listing, &root).unwrap();
        assert_eq!(installed.manifest.id, "acme.cam");
        let source = remote::source_of(&installed).unwrap();
        assert_eq!(
            (source.repo.as_str(), source.tag.as_str()),
            ("acme/printcad-cam", "v0.3.0")
        );

        // Not what the index says it is: refused, nothing replaced.
        let wrong = read_index(index(release(&sha256(b"other"), "printcad:workbench@0.1")))
            .unwrap()
            .packages
            .remove(0);
        assert!(
            install_listed(&Serves(bytes), &wrong, &root)
                .unwrap_err()
                .contains("checksum")
        );
        let other = archive("acme.other");
        let listing = read_index(index(release(&sha256(&other), "printcad:workbench@0.1")))
            .unwrap()
            .packages
            .remove(0);
        assert!(
            install_listed(&Serves(other), &listing, &root)
                .unwrap_err()
                .contains("not acme.cam")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
