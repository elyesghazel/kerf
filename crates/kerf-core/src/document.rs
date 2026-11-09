//! [`Document`] wraps a [`Project`] with undo/redo, built entirely out of the
//! inverse commands that [`Command::apply`] hands back.

use crate::command::{Command, EditError};
use crate::ids::IdGen;
use crate::project::Project;

const MAX_UNDO: usize = 1000;

/// An editable project plus its history.
#[derive(Debug, Clone, Default)]
pub struct Document {
    project: Project,
    undo: Vec<Command>,
    redo: Vec<Command>,
    /// Bumped on every change, so viewers know when to refresh / autosave.
    revision: u64,
    /// Key of the last coalescable edit (see [`Document::apply_merge`]).
    merge_key: Option<String>,
}

impl Document {
    pub fn new(project: Project) -> Self {
        Document {
            project,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            merge_key: None,
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn into_project(self) -> Project {
        self.project
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The id generator. Allocating ids is bookkeeping, not an undoable edit.
    pub fn ids(&mut self) -> &mut IdGen {
        &mut self.project.ids
    }

    /// Apply a command, recording its inverse for undo. Applying a fresh edit
    /// clears the redo stack, as in any editor.
    pub fn apply(&mut self, cmd: Command) -> Result<(), EditError> {
        self.merge_key = None;
        let inverse = cmd.apply(&mut self.project)?;
        self.undo.push(inverse);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision += 1;
        Ok(())
    }

    /// Apply a command that continues the previous one with the same `key`
    /// (a slider being dragged): the whole gesture becomes one undo step.
    /// Only valid for commands whose inverse restores the full prior state of
    /// what they touch (`UpdateClip`, `UpdateTrack`, `SetFormat`, ...).
    pub fn apply_merge(&mut self, cmd: Command, key: &str) -> Result<(), EditError> {
        if self.merge_key.as_deref() == Some(key) && !self.undo.is_empty() {
            cmd.apply(&mut self.project)?;
            self.redo.clear();
            self.revision += 1;
            return Ok(());
        }
        self.apply(cmd)?;
        self.merge_key = Some(key.to_string());
        Ok(())
    }

    /// End any merge gesture, so the next `apply_merge` starts a new step.
    pub fn end_merge(&mut self) {
        self.merge_key = None;
    }

    /// Apply several commands as one undo step.
    pub fn apply_all(&mut self, commands: Vec<Command>) -> Result<(), EditError> {
        match commands.len() {
            0 => Ok(()),
            1 => self.apply(commands.into_iter().next().unwrap()),
            _ => self.apply(Command::Batch { commands }),
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> Result<bool, EditError> {
        let Some(cmd) = self.undo.pop() else {
            return Ok(false);
        };
        // Undoing produces a redo command (the inverse of the inverse).
        self.merge_key = None;
        let redo = cmd.apply(&mut self.project)?;
        self.redo.push(redo);
        self.revision += 1;
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, EditError> {
        let Some(cmd) = self.redo.pop() else {
            return Ok(false);
        };
        let undo = cmd.apply(&mut self.project)?;
        self.undo.push(undo);
        self.revision += 1;
        Ok(true)
    }
}
