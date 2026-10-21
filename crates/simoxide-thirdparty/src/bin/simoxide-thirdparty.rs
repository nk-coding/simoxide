//! `simoxide-thirdparty`: the third-party test models (third_party/README.md).
//!
//! ```text
//! simoxide-thirdparty setup            derive the modified files (after `git submodule update --init`)
//! simoxide-thirdparty check [PATH..]   exit 1 with a message if material under PATH is unavailable
//! simoxide-thirdparty record DEST SRC  store DEST (a modified copy of SRC) as SRC + ed script
//! simoxide-thirdparty verify           every derived file equals SRC + script, every link resolves
//! ```
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use simoxide_thirdparty::{
    Derived, MANIFEST, SETUP_HINT, apply_ed, check, ensure, manifest, workspace_root,
};

const IGNORE_BEGIN: &str =
    "# >>> third_party derived files (simoxide-thirdparty record; do not edit)";
const IGNORE_END: &str = "# <<< third_party derived files";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("setup") => setup(),
        Some("check") => {
            let paths: Vec<PathBuf> = if args.len() > 1 {
                args[1..].iter().map(PathBuf::from).collect()
            } else {
                vec![".".into()]
            };
            paths.iter().try_for_each(|p| check(&abs(p)))
        }
        Some("record") if args.len() == 3 => record(Path::new(&args[1]), Path::new(&args[2])),
        Some("verify") => verify(),
        _ => Err(
            "usage: simoxide-thirdparty setup | check [PATH..] | record DEST SRC | verify".into(),
        ),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("simoxide-thirdparty: {e}");
            ExitCode::FAILURE
        }
    }
}

fn abs(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap().join(p)
    }
}

/// Relative to the root (accepts paths relative to the current directory).
fn rel(p: &Path) -> Result<PathBuf, String> {
    let root = workspace_root().canonicalize().map_err(|e| e.to_string())?;
    let a = abs(p);
    let parent = a
        .parent()
        .ok_or("bad path")?
        .canonicalize()
        .map_err(|e| format!("{}: {e}", p.display()))?;
    let full = parent.join(a.file_name().ok_or("bad path")?);
    full.strip_prefix(&root)
        .map(Path::to_path_buf)
        .map_err(|_| format!("{} is outside the workspace", p.display()))
}

fn setup() -> Result<(), String> {
    let failed = ensure();
    let n = manifest().map_err(|e| e.to_string())?.len();
    if failed.is_empty() {
        println!("derived {n} files ({MANIFEST})");
        Ok(())
    } else {
        for (d, e) in failed {
            eprintln!("  {}: {e}", d.display());
        }
        Err(format!(
            "{} of {n} derived files unavailable: {SETUP_HINT}",
            failed.len()
        ))
    }
}

fn record(dest: &Path, src: &Path) -> Result<(), String> {
    let root = workspace_root();
    let (dest, src) = (rel(dest)?, rel(src)?);
    if simoxide_thirdparty::submodule_of(&src).is_none() {
        return Err(format!("{} is not inside a submodule", src.display()));
    }
    let d = Derived {
        dest: dest.clone(),
        source: src.clone(),
    };
    let out = Command::new("diff")
        .arg("-e")
        .arg(root.join(&src))
        .arg(root.join(&dest))
        .output()
        .map_err(|e| format!("diff: {e}"))?;
    if out.status.code() == Some(2) {
        return Err(format!(
            "diff failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let script = String::from_utf8(out.stdout).map_err(|e| e.to_string())?;
    let want = fs::read_to_string(root.join(&dest)).map_err(|e| e.to_string())?;
    let orig = fs::read_to_string(root.join(&src)).map_err(|e| e.to_string())?;
    if apply_ed(&orig, &script).as_deref() != Ok(want.as_str()) {
        return Err(format!(
            "{}: the ed script does not reproduce the file (final newline or a `.` line?)",
            dest.display()
        ));
    }
    let sp = root.join(d.script());
    fs::create_dir_all(sp.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&sp, &script).map_err(|e| e.to_string())?;
    // manifest
    let mp = root.join(MANIFEST);
    let text = fs::read_to_string(&mp).unwrap_or_default();
    let mut header: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with('#') || l.is_empty())
        .map(String::from)
        .collect();
    while header.last().is_some_and(String::is_empty) {
        header.pop();
    }
    let mut entries = simoxide_thirdparty::parse_manifest(&text).map_err(|e| e.to_string())?;
    entries.retain(|e| e.dest != dest);
    entries.push(d);
    entries.sort_by(|a, b| a.dest.cmp(&b.dest));
    let mut out = header.join("\n");
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    for e in &entries {
        out.push_str(&format!("{}\t{}\n", e.dest.display(), e.source.display()));
    }
    fs::write(&mp, out).map_err(|e| e.to_string())?;
    update_gitignore(&entries)?;
    println!(
        "{}: {} + {} ({} bytes)",
        dest.display(),
        src.display(),
        entries
            .iter()
            .find(|e| e.dest == dest)
            .unwrap()
            .script()
            .display(),
        script.len()
    );
    Ok(())
}

fn update_gitignore(entries: &[Derived]) -> Result<(), String> {
    let p = workspace_root().join(".gitignore");
    let text = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let mut block = format!("{IGNORE_BEGIN}\n");
    for e in entries {
        block.push_str(&format!("/{}\n", e.dest.display()));
    }
    block.push_str(IGNORE_END);
    let new = match (text.find(IGNORE_BEGIN), text.find(IGNORE_END)) {
        (Some(a), Some(b)) => format!("{}{}{}", &text[..a], block, &text[b + IGNORE_END.len()..]),
        _ => format!("{}\n{}\n", text.trim_end(), block),
    };
    fs::write(&p, new).map_err(|e| e.to_string())
}

fn verify() -> Result<(), String> {
    let root = workspace_root();
    let mut bad = 0;
    for d in manifest().map_err(|e| e.to_string())? {
        let orig = fs::read_to_string(root.join(&d.source))
            .map_err(|_| format!("{}: source missing: {SETUP_HINT}", d.source.display()))?;
        let script = fs::read_to_string(root.join(d.script()))
            .map_err(|e| format!("{}: {e}", d.script().display()))?;
        let want = apply_ed(&orig, &script)?;
        match fs::read_to_string(root.join(&d.dest)) {
            Ok(t) if t == want => {}
            Ok(_) => {
                eprintln!(
                    "differs: {} (edited by hand? record it again, or run setup)",
                    d.dest.display()
                );
                bad += 1;
            }
            Err(_) => {
                eprintln!("missing: {} (run setup)", d.dest.display());
                bad += 1;
            }
        }
    }
    check(&root)?;
    if bad > 0 {
        Err(format!("{bad} derived files out of date"))
    } else {
        Ok(())
    }
}
