//! Resource-set configuration shared with the Java oracle (`oracle/src/EmfOracle.java`): URI
//! prefix maps, an index of metamodel files by namespace URI, package-registry aliases.
//!
//! Format (one directive per line, paths relative to a base directory):
//!
//! ```text
//! map <from-uri-prefix> <path-prefix>
//! mmroot <dir>            # index the nsURIs of all .ecore files below <dir>
//! alias <key> <nsURI>
//! ```

use crate::resource_set::ResourceSet;
use crate::uri::Uri;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub maps: Vec<(String, String)>,
    /// nsURI -> file URI of the first `.ecore` (in path order) declaring it.
    pub ns_index: BTreeMap<String, String>,
    pub aliases: Vec<(String, String)>,
}

fn file_uri(p: &Path) -> String {
    Uri::file(&p.to_string_lossy()).to_string()
}

impl Config {
    /// Adds what `other` has and `self` does not (maps of other prefixes, namespace URIs,
    /// aliases): a configuration on top of a base one.
    pub fn with_base(mut self, other: &Config) -> Config {
        for (k, v) in &other.maps {
            if !self.maps.iter().any(|(a, _)| a == k) {
                self.maps.push((k.clone(), v.clone()));
            }
        }
        for (k, v) in &other.ns_index {
            self.ns_index.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for (k, v) in &other.aliases {
            if !self.aliases.iter().any(|(a, _)| a == k) {
                self.aliases.push((k.clone(), v.clone()));
            }
        }
        self
    }

    pub fn read(conf: &Path, base: &Path) -> std::io::Result<Config> {
        let text = std::fs::read_to_string(conf)?;
        let mut c = Config::default();
        let mut roots = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let p: Vec<&str> = line.split_whitespace().collect();
            match p[0] {
                "map" => {
                    let abs = std::path::absolute(base.join(p[2]))?;
                    let mut to = file_uri(&abs);
                    if p[1].ends_with('/') && !to.ends_with('/') {
                        to.push('/');
                    }
                    c.maps.push((p[1].to_string(), to));
                }
                "mmroot" => roots.push(base.join(p[1])),
                "alias" => c.aliases.push((p[1].to_string(), p[2].to_string())),
                _ => {}
            }
        }
        let mut files = Vec::new();
        for r in roots {
            collect_ecore(&r, &mut files);
        }
        files.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
        for f in files {
            let abs = std::path::absolute(&f)?;
            let loc = c.unmap(&file_uri(&abs));
            for ns in ns_uris_of_file(&f) {
                c.ns_index.entry(ns).or_insert_with(|| loc.clone());
            }
        }
        Ok(c)
    }

    /// The URI under which a file is known: a mapped URI (`platform:/plugin/...`) if a prefix
    /// mapping leads to it (the first such mapping), else the file URI. Metamodels then refer
    /// to each other through the mapped URIs, as in Eclipse.
    pub fn unmap(&self, file_uri: &str) -> String {
        for (from, to) in &self.maps {
            if from.ends_with('/') && file_uri.starts_with(to.as_str()) {
                return format!("{from}{}", &file_uri[to.len()..]);
            }
        }
        file_uri.to_string()
    }

    /// A resource set for loading the model at `model`: this configuration, with the
    /// metamodels in the model's own directory taking precedence over the global index.
    pub fn resource_set_for(&self, model: &Path) -> ResourceSet {
        let mut rs = ResourceSet::new();
        for (from, to) in &self.maps {
            rs.map_uri(from, to);
        }
        for (key, ns) in &self.aliases {
            rs.add_alias(key, ns);
        }
        for (ns, loc) in &self.ns_index {
            rs.add_ns_location(ns, loc);
        }
        if let Some(dir) = std::path::absolute(model)
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            && let Ok(rd) = std::fs::read_dir(&dir)
        {
            let mut local: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "ecore") && p.is_file())
                .collect();
            local.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
            let mut seen = std::collections::HashSet::new();
            for f in local {
                for ns in ns_uris_of_file(&f) {
                    if seen.insert(ns.clone()) {
                        rs.add_ns_location(&ns, &self.unmap(&file_uri(&f)));
                    }
                }
            }
        }
        rs
    }
}

fn collect_ecore(d: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != ".git") {
                collect_ecore(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "ecore") {
            out.push(p);
        }
    }
}

/// The nsURIs declared by the `EPackage` / `eSubpackages` start tags of an `.ecore` file.
pub fn ns_uris_of_file(f: &Path) -> Vec<String> {
    std::fs::read_to_string(f)
        .map(|t| ns_uris(&t))
        .unwrap_or_default()
}

pub fn ns_uris(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(k) = memchr::memchr(b'<', &b[i..]) {
        let start = i + k + 1;
        let mut j = start;
        while j < b.len()
            && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b':' | b'.' | b'-'))
        {
            j += 1;
        }
        let name = &text[start..j];
        let local = name.rsplit(':').next().unwrap_or(name);
        let tag_end = memchr::memchr(b'>', &b[j..]).map_or(b.len(), |e| j + e);
        if (local == "EPackage" || local == "eSubpackages")
            && (j == b.len() || !b[j].is_ascii_alphanumeric())
        {
            let tag = &text[j..tag_end];
            let tb = tag.as_bytes();
            let mut p = 0;
            while let Some(q) = tag[p..].find("nsURI=\"") {
                let at = p + q;
                if at > 0 && tb[at - 1].is_ascii_whitespace() {
                    let vs = at + 7;
                    if let Some(ve) = tag[vs..].find('"') {
                        out.push(tag[vs..vs + ve].replace("&amp;", "&"));
                    }
                    break;
                }
                p = at + 1;
            }
        }
        i = start;
        let _ = tag_end;
    }
    out
}
