//! Turns what the user typed in the address bar into a folder to open.

use std::path::{Path, PathBuf};

/// `~`, `%VAR%`, quotes, a bare drive (`D:`) and a path to a file (opens its folder) are all
/// understood. `None` when nothing like it exists.
pub fn resolve(input: &str) -> Option<PathBuf> {
    let text = input.trim().trim_matches('"').trim();
    if text.is_empty() {
        return None;
    }
    let text = expand(text);
    let mut path = PathBuf::from(&text);
    // "D:" alone means "the current folder of D:"; the root is what is meant.
    let b = text.as_bytes();
    if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        path = PathBuf::from(format!("{text}\\"));
    }
    existing_folder(&path)
}

/// Folders that could complete what is being typed: the sub-folders of the part before the last
/// `\` (or `/`) whose names start with the part after it, case-insensitively, sorted by name.
/// `C:` alone lists the drive's root. At most `limit` entries.
pub fn suggest(input: &str, limit: usize) -> Vec<PathBuf> {
    let text = expand(input.trim().trim_matches('"'));
    let (dir, prefix) = match text.rfind(['\\', '/']) {
        Some(i) => (text[..=i].to_owned(), text[i + 1..].to_owned()),
        None => {
            let b = text.as_bytes();
            if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
                (format!("{text}\\"), String::new())
            } else {
                return Vec::new();
            }
        }
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let prefix = prefix.to_lowercase();
    let mut names: Vec<String> = entries
        .flatten()
        .take(20_000)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_lowercase().starts_with(&prefix))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.truncate(limit);
    names.into_iter().map(|n| Path::new(&dir).join(n)).collect()
}

fn existing_folder(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        Some(path.to_path_buf())
    } else if path.is_file() {
        path.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// Leading `~` and `%NAME%` references.
fn expand(text: &str) -> String {
    let mut s = text.to_owned();
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            s = format!("{}{}", home.display(), &s[1..]);
        }
    }
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => match std::env::var(&after[..end]) {
                Ok(v) => {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_missing_are_none() {
        assert_eq!(resolve("   "), None);
        assert_eq!(resolve("/definitely/not/here/qf"), None);
    }

    #[test]
    fn folders_files_and_quotes() {
        let dir = std::env::temp_dir().join(format!("qf-pathinput-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(resolve(&dir.to_string_lossy()), Some(dir.clone()));
        assert_eq!(
            resolve(&format!("  \"{}\" ", dir.display())),
            Some(dir.clone())
        );
        assert_eq!(resolve(&file.to_string_lossy()), Some(dir.clone()));
        let _ = std::fs::remove_dir_all(&dir);
        // A variable that exists on every machine.
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        if let Some(home) = std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|h| h.is_dir())
        {
            assert_eq!(resolve(&format!("%{var}%")), Some(home));
        }
    }

    #[test]
    fn unknown_percent_is_kept() {
        assert_eq!(expand("100%"), "100%");
        assert_eq!(expand("%QF_NOPE_X%"), "%QF_NOPE_X%");
    }

    #[test]
    fn suggests_matching_subfolders() {
        let root = std::env::temp_dir().join(format!("qf-suggest-{}", std::process::id()));
        for d in ["Alpha", "alps", "Beta"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("alpha.txt"), b"x").unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        let typed = format!("{}{sep}AL", root.display());
        let names: Vec<String> = suggest(&typed, 10)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Alpha", "alps"], "folders only, any case");
        let all = suggest(&format!("{}{sep}", root.display()), 2);
        assert_eq!(all.len(), 2, "limit");
        assert!(suggest("nothing-like-a-path", 5).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
