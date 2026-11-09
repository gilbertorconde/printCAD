// Workspace metadata and change ownership shared by the repository gates.
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));

export function workspaceAt(directory, run = execFileSync, exists = existsSync) {
  directory = resolve(directory);
  const command = (program, args) => run(program, args, {
    cwd: directory, encoding: "utf8", maxBuffer: 1 << 28,
  });
  let packages;
  const changes = new Map();
  const owners = new Map();
  const portable = (path) => path.replaceAll("\\", "/");

  function members() {
    if (!packages) {
      const metadata = JSON.parse(command("cargo", ["metadata", "--format-version", "1", "--no-deps"]));
      const ids = new Set(metadata.workspace_members);
      packages = metadata.packages.filter(p => ids.has(p.id)).map(p => ({
        ...p,
        manifest: p.manifest_path,
        dir: portable(relative(directory, dirname(p.manifest_path))),
        deps: p.dependencies.filter(d => d.path).map(d => d.name),
      }));
    }
    return packages;
  }

  function ownerOf(file) {
    const absolute = resolve(directory, file);
    if (owners.has(absolute)) return owners.get(absolute);
    const byManifest = new Map(members().map(p => [p.manifest, p]));
    let dir = dirname(absolute);
    let owner = null;
    while (dir !== directory && dir !== dirname(dir)) {
      const manifest = join(dir, "Cargo.toml");
      if (byManifest.has(manifest) || exists(manifest)) {
        // Excluded packages, such as the guest SDK, still own their files.
        owner = byManifest.get(manifest) ?? { manifest, dir: portable(relative(directory, dir)) };
        break;
      }
      dir = dirname(dir);
    }
    owners.set(absolute, owner);
    return owner;
  }

  function changedSince(ref = "HEAD") {
    if (!changes.has(ref)) {
      const base = ref === "HEAD" ? ref : command("git", ["merge-base", ref, "HEAD"]).trim();
      const files = new Set([
        ...command("git", ["diff", "--name-only", "-z", base]).split("\0"),
        ...command("git", ["ls-files", "-z", "--others", "--exclude-standard"]).split("\0"),
      ].filter(Boolean));
      changes.set(ref, { base, files });
    }
    return changes.get(ref);
  }
  return { members, ownerOf, changedSince };
}

export const { members, ownerOf, changedSince } = workspaceAt(root);
