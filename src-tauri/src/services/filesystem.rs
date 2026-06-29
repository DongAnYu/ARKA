use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::models::note::Note;

pub fn select_vault(vault_path: &str) -> Result<PathBuf, String> {
    let path: PathBuf = PathBuf::from(vault_path);

    if !path.exists() {
        return Err(format!("Vault path does not exist: {vault_path}"));
    }

    if !path.is_dir() {
        return Err(format!("Vault path is not a directory: {vault_path}"));
    }

    path.canonicalize()
        .map_err(|err| format!("Failed to resolve vault path: {err}"))
}

pub fn scan_markdown_files(vault_path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut markdown_files = Vec::new();
    scan_recursive(vault_path, &mut markdown_files)?;
    Ok(markdown_files)
}

pub fn load_vault_notes(vault_path: &str) -> Result<Vec<Note>, String> {
    let vault = select_vault(vault_path)?;

    let files =
        scan_markdown_files(&vault).map_err(|err| format!("Failed to scan vault: {err}"))?;

    let mut notes = Vec::new();
    for file in files {
        let note = read_note(&file)
            .map_err(|err| format!("Failed to read note {}: {err}", file.to_string_lossy()))?;
        notes.push(note);
    }

    Ok(notes)
}

pub fn read_note(path: &Path) -> io::Result<Note> {
    let content = fs::read_to_string(path)?;
    let metadata = fs::metadata(path)?;

    let last_modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|| String::from("0"));

    Ok(Note {
        id: None,
        path: path.to_string_lossy().to_string(),
        title: extract_title(path),
        content,
        last_modified,
    })
}

fn scan_recursive(dir: &Path, markdown_files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            scan_recursive(&path, markdown_files)?;
            continue;
        }

        let is_markdown = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false);

        if is_markdown {
            markdown_files.push(path);
        }
    }

    Ok(())
}

fn extract_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
        .unwrap_or_else(|| String::from("Untitled"))
}
