//! Workbench packages on disk: a folder holding `bench.toml`, `bench.wasm`
//! and `icons/`, installed from a `.pcbench` archive (a tar file, gzipped
//! or not, of the same files).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use bench_api::Manifest;

pub const MANIFEST: &str = "bench.toml";
pub const COMPONENT: &str = "bench.wasm";
pub const ARCHIVE_EXTENSION: &str = "pcbench";

/// An installed package.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Package {
    /// The package in `dir`, checked: its manifest reads, names a contract
    /// this host speaks, and its component is there.
    pub fn read(dir: &Path) -> Result<Package, String> {
        let text = fs::read_to_string(dir.join(MANIFEST))
            .map_err(|e| format!("cannot read {MANIFEST}: {e}"))?;
        let manifest = parse_manifest(&text)?;
        if !dir.join(COMPONENT).is_file() {
            return Err(format!("{} has no {COMPONENT}", manifest.id));
        }
        Ok(Package {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    pub fn wasm(&self) -> PathBuf {
        self.dir.join(COMPONENT)
    }

    /// Where the app keeps the package's compiled component: beside the
    /// installed packages rather than in one, where no archive reaches.
    pub fn compiled_dir(&self) -> PathBuf {
        self.dir.parent().unwrap_or(&self.dir).join(".compiled")
    }

    /// Where the package keeps its own files; the only folder it reaches.
    pub fn data_dir(&self) -> PathBuf {
        self.dir.join("data")
    }

    /// The package's icons: `icons/<name>.svg`, by name.
    pub fn icons(&self) -> Vec<(String, String)> {
        let Ok(entries) = fs::read_dir(self.dir.join("icons")) else {
            return Vec::new();
        };
        let mut icons: Vec<(String, String)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != "svg" {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                Some((name, fs::read_to_string(&path).ok()?))
            })
            .collect();
        icons.sort();
        icons
    }
}

/// A manifest from its text, checked.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let manifest: Manifest =
        toml::from_str(text).map_err(|e| format!("{MANIFEST} does not read: {e}"))?;
    let id_ok = !manifest.id.is_empty()
        && manifest.id.contains('.')
        && manifest
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'));
    if !id_ok {
        return Err(format!(
            "`{}` is not a package id: lowercase, reverse-domain, such as `acme.cam`",
            manifest.id
        ));
    }
    if !bench_api::api_compatible(&manifest.api) {
        return Err(format!(
            "{} targets {}, and this app speaks printcad:workbench@{}",
            manifest.id,
            manifest.api,
            bench_api::API_VERSION
        ));
    }
    for kind in &manifest.feature_kinds {
        if !kind.starts_with(&format!("{}.", manifest.id)) {
            return Err(format!(
                "feature kind `{kind}` must start with the package id `{}.`",
                manifest.id
            ));
        }
    }
    if manifest.memory_mb == 0 || manifest.memory_mb > 4096 {
        return Err("memory_mb must be between 1 and 4096".into());
    }
    Ok(manifest)
}

/// Every package folder under `root`, read, in name order; one that does
/// not read comes with its folder and the reason.
pub fn discover(root: &Path) -> Vec<Result<Package, (PathBuf, String)>> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(MANIFEST).is_file())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .map(|dir| Package::read(&dir).map_err(|e| (dir, e)))
        .collect()
}

/// Install the archive at `archive` under `root`, replacing an installed
/// version of the same package but keeping its data folder.
pub fn install(archive: &Path, root: &Path) -> Result<Package, String> {
    let bytes = fs::read(archive).map_err(|e| format!("cannot read {}: {e}", archive.display()))?;
    install_bytes(&bytes, root)
}

/// [`install`] from the archive's bytes.
pub fn install_bytes(bytes: &[u8], root: &Path) -> Result<Package, String> {
    fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {e}", root.display()))?;
    let staging = root.join(format!(".installing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let unpacked = unpack(bytes, &staging).and_then(|_| Package::read(&staging));
    let package = match unpacked {
        Ok(package) => package,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let target = root.join(&package.manifest.id);
    if target.exists() {
        let old_data = target.join("data");
        if old_data.is_dir() {
            let _ = fs::remove_dir_all(staging.join("data"));
            let _ = fs::rename(&old_data, staging.join("data"));
        }
        fs::remove_dir_all(&target)
            .map_err(|e| format!("cannot replace the installed version: {e}"))?;
    }
    fs::rename(&staging, &target).map_err(|e| format!("cannot install: {e}"))?;
    Package::read(&target)
}

fn unpack(bytes: &[u8], into: &Path) -> Result<(), String> {
    let reader: Box<dyn Read + '_> = if bytes.starts_with(&[0x1f, 0x8b]) {
        Box::new(flate2::read::GzDecoder::new(bytes))
    } else {
        Box::new(bytes)
    };
    let mut archive = tar::Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|e| format!("not a package archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("the archive is damaged: {e}"))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("the archive is damaged: {e}"))?
            .into_owned();
        if !belongs_in_package(&path) {
            continue;
        }
        // `unpack_in` refuses a path that would land outside `into`.
        let inside = entry
            .unpack_in(into)
            .map_err(|e| format!("cannot unpack: {e}"))?;
        if !inside {
            return Err("the archive reaches outside its folder".into());
        }
    }
    Ok(())
}

/// Whether an archive entry is one a package holds: its manifest, its
/// component, its readme, its icons and helpers, nothing hidden.
fn belongs_in_package(path: &Path) -> bool {
    use std::path::Component;
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let Some(first) = parts.first() else {
        return false;
    };
    let top = matches!(
        first.as_str(),
        MANIFEST | COMPONENT | "README.md" | "icons" | "helpers"
    );
    top && parts.iter().all(|p| !p.starts_with('.'))
}

/// Remove the installed package `id`, its data with it.
pub fn uninstall(root: &Path, id: &str) -> Result<(), String> {
    let dir = root.join(id);
    if !dir.join(MANIFEST).is_file() {
        return Err(format!("{id} is not installed"));
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("cannot remove {id}: {e}"))?;
    if let Ok(entries) = fs::read_dir(root.join(".compiled")) {
        let prefix = format!("{id}@");
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

/// Pack the package folder `dir` into a gzipped `.pcbench` archive at
/// `out`: the manifest, the component, `icons/`, `helpers/` and a
/// `README.md` when there is one.
pub fn pack(dir: &Path, out: &Path) -> Result<Manifest, String> {
    let package = Package::read(dir)?;
    let file = fs::File::create(out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.mode(tar::HeaderMode::Deterministic);
    for name in [MANIFEST, COMPONENT, "README.md"] {
        let path = dir.join(name);
        if path.is_file() {
            tar.append_path_with_name(&path, name)
                .map_err(|e| e.to_string())?;
        }
    }
    for folder in ["icons", "helpers"] {
        let path = dir.join(folder);
        if path.is_dir() {
            tar.append_dir_all(folder, &path)
                .map_err(|e| e.to_string())?;
        }
    }
    tar.into_inner()
        .and_then(|gz| gz.finish())
        .map_err(|e| e.to_string())?;
    Ok(package.manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
id = "acme.cam"
name = "CAM"
version = "0.3.0"
api = "printcad:workbench@0.1"
feature_kinds = ["acme.cam.pocket"]
[capabilities]
helper = true
"#;

    #[test]
    fn an_archive_unpacks_only_what_a_package_holds() {
        for (path, kept) in [
            ("bench.toml", true),
            ("./bench.wasm", true),
            ("icons/gear.svg", true),
            ("helpers/linux-x86_64/cut", true),
            (".compiled/x.cwasm", false),
            ("icons/.hidden.svg", false),
            ("data/seed.txt", false),
            ("other.bin", false),
        ] {
            assert_eq!(belongs_in_package(Path::new(path)), kept, "{path}");
        }
    }

    #[test]
    fn a_manifest_reads_with_its_capabilities_and_defaults() {
        let m = parse_manifest(GOOD).unwrap();
        assert_eq!(m.id, "acme.cam");
        assert!(m.capabilities.helper && !m.capabilities.network);
        assert_eq!(m.memory_mb, 1024);
    }

    #[test]
    fn a_manifest_that_would_clash_or_cannot_run_is_refused() {
        let refused = |from: &str, to: &str| parse_manifest(&GOOD.replace(from, to)).unwrap_err();
        assert!(refused("acme.cam.pocket", "part.pocket").contains("must start with"));
        assert!(refused("@0.1", "@0.2").contains("speaks"));
        assert!(refused("id = \"acme.cam\"", "id = \"Acme CAM\"").contains("not a package id"));
        assert!(
            refused("[capabilities]", "memory_mb = 9000\n[capabilities]").contains("memory_mb")
        );
    }
}
