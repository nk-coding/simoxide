//! Finding units on the file system: imports relative to the importing file's folder (QVTo's
//! standalone `URIUnitResolver`) or against the source containers of bundle directories (as
//! `PlatformPluginUnitResolver` resolves `platform:/plugin` units under OSGi, emulated by the
//! Java reference runner's `BundleUnitResolverFactory`).

use qvto_sema::{UnitResolver, UnitSource};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A bundle directory with the source containers of its `plugin.xml`.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub root: PathBuf,
    /// Container paths relative to the root (`""` for the root itself).
    pub containers: Vec<String>,
}

/// Resolves units by qualified name on the file system.
#[derive(Clone, Debug, Default)]
pub struct FsResolver {
    pub bundles: Vec<Bundle>,
    /// Only the importing file's folder (`imports folder` cases).
    pub from_folder: bool,
}

impl FsResolver {
    /// Imports relative to the importing file's folder.
    pub fn folder() -> FsResolver {
        FsResolver {
            bundles: Vec::new(),
            from_folder: true,
        }
    }

    /// Bundles: the directories below `parent` with a `plugin.xml`, with the `sourceContainer`
    /// paths of their `qvtTransformationContainer` extensions (the root if none).
    pub fn bundles_below(parent: &Path) -> Vec<Bundle> {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(parent)
            .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        dirs.sort();
        let mut out = Vec::new();
        for d in dirs {
            let Ok(px) = std::fs::read_to_string(d.join("plugin.xml")) else {
                continue;
            };
            let mut containers = Vec::new();
            let mut rest = px.as_str();
            while let Some(i) = rest.find("<sourceContainer") {
                rest = &rest[i..];
                let end = rest.find('>').unwrap_or(rest.len());
                let tag = &rest[..end];
                if let Some(p) = attr(tag, "path") {
                    let p = p.trim();
                    if !p.is_empty() {
                        containers.push(if p == "/" {
                            String::new()
                        } else {
                            p.trim_matches('/').to_string()
                        });
                    }
                }
                rest = &rest[end..];
            }
            if containers.is_empty() {
                containers.push(String::new());
            }
            let root = d.canonicalize().unwrap_or(d);
            out.push(Bundle { root, containers });
        }
        out
    }

    fn bundle_of(&self, path: &Path) -> Option<&Bundle> {
        if self.from_folder {
            return None;
        }
        let p = path.canonicalize().ok()?;
        self.bundles.iter().find(|b| p.starts_with(&b.root))
    }

    /// The qualified name of a unit: its path below its bundle (minus the first matching
    /// container) or its file stem, with dots.
    pub fn qualified_name(&self, path: &Path) -> String {
        if let Some(b) = self.bundle_of(path)
            && let Ok(p) = path.canonicalize()
            && let Ok(rel) = p.strip_prefix(&b.root)
        {
            let mut rel = rel.with_extension("").to_string_lossy().replace('\\', "/");
            for c in &b.containers {
                if c.is_empty() {
                    continue;
                }
                if let Some(r) = rel.strip_prefix(&format!("{c}/")) {
                    rel = r.to_string();
                    break;
                }
            }
            return rel.replace('/', ".");
        }
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The unit of a file.
    pub fn unit_source(&self, path: &Path) -> std::io::Result<UnitSource> {
        let text = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&text).into_owned();
        let key = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        Ok(UnitSource {
            key,
            qualified_name: self.qualified_name(path),
            file_name: path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: Some(path.to_path_buf()),
            text: Rc::from(text.as_str()),
        })
    }
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

impl UnitResolver for FsResolver {
    fn resolve(&self, importer: &UnitSource, qualified_name: &str) -> Option<UnitSource> {
        let rel = format!("{}.qvto", qualified_name.replace('.', "/"));
        let ipath = importer.path.as_deref()?;
        if let Some(b) = self.bundle_of(ipath) {
            // the bundle root as the importer's path spells it
            let root = match (ipath.canonicalize().ok(), ipath.parent()) {
                (Some(c), _) => {
                    let below = c.strip_prefix(&b.root).ok()?;
                    let depth = below.components().count();
                    let mut r = ipath.to_path_buf();
                    for _ in 0..depth {
                        r.pop();
                    }
                    r
                }
                _ => b.root.clone(),
            };
            for c in &b.containers {
                let base = if c.is_empty() {
                    root.clone()
                } else {
                    root.join(c)
                };
                let p = base.join(&rel);
                if p.is_file() {
                    return self.unit_source(&p).ok();
                }
            }
            return None;
        }
        let p = ipath.parent()?.join(&rel);
        if p.is_file() {
            return self.unit_source(&p).ok();
        }
        None
    }
}
