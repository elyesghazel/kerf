//! Undo across CLI invocations: a sidecar `<project>.kerf-history.json`
//! holding whole-project snapshots (projects are tiny edit lists).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use kerf_core::Project;
use serde::{Deserialize, Serialize};

const CAP: usize = 200;

#[derive(Default, Serialize, Deserialize)]
struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}

fn sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".kerf-history.json");
    PathBuf::from(s)
}

fn read(path: &Path) -> History {
    std::fs::read_to_string(sidecar(path))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write(path: &Path, h: &History) -> Result<()> {
    let f = sidecar(path);
    let tmp = f.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(h)?)?;
    std::fs::rename(&tmp, &f).map_err(|e| anyhow!(e))
}

/// Record the state before an edit.
pub fn push(path: &Path, before: &Project) -> Result<()> {
    let mut h = read(path);
    h.undo.push(before.clone());
    if h.undo.len() > CAP {
        h.undo.remove(0);
    }
    h.redo.clear();
    write(path, &h)
}

pub fn undo(path: &Path, current: &Project) -> Result<Option<Project>> {
    let mut h = read(path);
    let Some(prev) = h.undo.pop() else { return Ok(None) };
    h.redo.push(current.clone());
    write(path, &h)?;
    Ok(Some(prev))
}

pub fn redo(path: &Path, current: &Project) -> Result<Option<Project>> {
    let mut h = read(path);
    let Some(next) = h.redo.pop() else { return Ok(None) };
    h.undo.push(current.clone());
    write(path, &h)?;
    Ok(Some(next))
}

pub fn clear(path: &Path) {
    let _ = std::fs::remove_file(sidecar(path));
}
