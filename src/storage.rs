//! On-disk persistence following the XDG base directory spec (via the `directories` crate, which
//! honours `$XDG_DATA_HOME`, `$XDG_CACHE_HOME`, ...).
//!
//! * `$XDG_DATA_HOME/kanban/board.json`   – the board itself (the only thing that matters).
//! * `$XDG_CACHE_HOME/kanban/board.json.bak` – copy of the board as it was when this session
//!   first saved. Disposable, but handy if something goes wrong.

use crate::model::Board;
use directories::ProjectDirs;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "board.json";

pub struct Storage {
    data_file: PathBuf,
    backup_file: PathBuf,
    backed_up: bool,
    /// Set when the existing file could not be read, so we never clobber data we couldn't see.
    save_blocked: bool,
}

impl Storage {
    pub fn from_xdg() -> Option<Self> {
        let dirs = ProjectDirs::from("", "", "kanban")?;
        Some(Self::new(dirs.data_dir(), dirs.cache_dir()))
    }

    pub fn new(data_dir: &Path, cache_dir: &Path) -> Self {
        Storage {
            data_file: data_dir.join(FILE_NAME),
            backup_file: cache_dir.join(format!("{FILE_NAME}.bak")),
            backed_up: false,
            save_blocked: false,
        }
    }

    #[cfg(test)]
    pub fn data_file(&self) -> &Path {
        &self.data_file
    }

    /// Loads the board, or a fresh default one if none exists yet.
    ///
    /// A file that cannot be parsed is never overwritten: it is renamed to
    /// `board.json.corrupt` and the returned warning says so.
    pub fn load(&mut self) -> (Board, Option<String>) {
        let text = match fs::read_to_string(&self.data_file) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (Board::default(), None),
            Err(e) => {
                self.save_blocked = true;
                return (
                    Board::default(),
                    Some(format!(
                        "Could not read {}: {e}. Changes will not be saved this session so the \
                         existing file is not overwritten.",
                        self.data_file.display()
                    )),
                );
            }
        };
        match serde_json::from_str::<Board>(&text) {
            Ok(mut board) => {
                board.repair();
                (board, None)
            }
            Err(e) => {
                let corrupt = self.data_file.with_extension("json.corrupt");
                let moved = fs::rename(&self.data_file, &corrupt);
                let msg = match moved {
                    Ok(()) => format!(
                        "{} was not valid ({e}); it was kept as {} and a new board was started.",
                        self.data_file.display(),
                        corrupt.display()
                    ),
                    Err(re) => format!(
                        "{} was not valid ({e}) and could not be set aside ({re}).",
                        self.data_file.display()
                    ),
                };
                (Board::default(), Some(msg))
            }
        }
    }

    /// Atomically writes the board (temp file + rename) so a crash cannot leave a torn file.
    pub fn save(&mut self, board: &Board) -> io::Result<()> {
        if self.save_blocked {
            return Err(io::Error::other(
                "saving is disabled (existing file was unreadable)",
            ));
        }
        if let Some(dir) = self.data_file.parent() {
            fs::create_dir_all(dir)?;
        }
        if !self.backed_up {
            // Best effort: a failed backup must not block saving.
            if self.data_file.exists()
                && let Some(dir) = self.backup_file.parent()
                && fs::create_dir_all(dir).is_ok()
            {
                let _ = fs::copy(&self.data_file, &self.backup_file);
            }
            self.backed_up = true;
        }
        let json = serde_json::to_string_pretty(board).map_err(io::Error::other)?;
        let tmp = self.data_file.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.data_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique scratch directory under the OS temp dir, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "kanban-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn storage(&self) -> Storage {
            Storage::new(&self.0.join("data"), &self.0.join("cache"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_file_gives_default_board() {
        let s = Scratch::new();
        let (board, warning) = s.storage().load();
        assert_eq!(board, Board::default());
        assert!(warning.is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let s = Scratch::new();
        let mut st = s.storage();
        let mut board = Board::default();
        let col = board.columns[0].id;
        let t = board.add_task(col, "write tests").unwrap();
        board.add_subtask(t, "sub");
        st.save(&board).unwrap();
        let (loaded, warning) = s.storage().load();
        assert_eq!(loaded, board);
        assert!(warning.is_none());
        assert!(!st.data_file().with_extension("json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_set_aside_not_overwritten() {
        let s = Scratch::new();
        let mut st = s.storage();
        fs::create_dir_all(st.data_file().parent().unwrap()).unwrap();
        fs::write(st.data_file(), "{ not json").unwrap();
        let (board, warning) = st.load();
        assert_eq!(board, Board::default());
        assert!(warning.unwrap().contains("board.json.corrupt"));
        let corrupt = st.data_file().with_extension("json.corrupt");
        assert_eq!(fs::read_to_string(corrupt).unwrap(), "{ not json");
        assert!(!st.data_file().exists());
    }

    #[test]
    fn first_save_of_a_session_backs_up_previous_file_to_cache() {
        let s = Scratch::new();
        let mut first = s.storage();
        let mut board = Board::default();
        first.save(&board).unwrap();
        let original = fs::read_to_string(first.data_file()).unwrap();

        let mut second = s.storage();
        let col = board.columns[0].id;
        board.add_task(col, "new");
        second.save(&board).unwrap();
        board.add_task(col, "newer");
        second.save(&board).unwrap();

        let backup = s.0.join("cache").join("board.json.bak");
        assert_eq!(fs::read_to_string(backup).unwrap(), original);
    }

    #[test]
    fn unreadable_file_blocks_saving() {
        let s = Scratch::new();
        let mut st = s.storage();
        // A directory where the file should be makes reading fail with a non-NotFound error.
        fs::create_dir_all(st.data_file()).unwrap();
        let (_, warning) = st.load();
        assert!(warning.is_some());
        assert!(st.save(&Board::default()).is_err());
        assert!(st.data_file().is_dir());
    }

    #[test]
    fn loading_repairs_inconsistent_data() {
        let s = Scratch::new();
        let mut st = s.storage();
        fs::create_dir_all(st.data_file().parent().unwrap()).unwrap();
        let json = r#"{"next_id":1,"columns":[{"id":10,"name":"A","tasks":[5]}],
            "tasks":{"3":{"id":3,"title":"x","depends_on":[42]}}}"#;
        fs::write(st.data_file(), json).unwrap();
        let (board, warning) = st.load();
        assert!(warning.is_none());
        assert_eq!(board.columns[0].tasks, [3]);
        assert!(board.tasks[&3].depends_on.is_empty());
        assert!(board.next_id > 10);
    }
}
