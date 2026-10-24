//! Availability fixture for GNU cl-lib's pre-generated-cl-loaddefs branch.
//! Only search-path availability changes; explicit owner loads use production GNU files.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SourceOnlyEmacsLisp {
    directory: PathBuf,
}

impl SourceOnlyEmacsLisp {
    pub fn new(root: &Path) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let fixture = loop {
            let directory = std::env::temp_dir().join(format!(
                "romacs-source-only-emacs-lisp-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&directory) {
                Ok(()) => break Self { directory },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create source-only GNU search directory: {error}"),
            }
        };
        // cl-lib.el deliberately requires cl-macs/cl-seq when cl-loaddefs is absent.
        // Never remove that generated file from the production GNU tree.
        for entry in fs::read_dir(root.join("gnu-lisp/emacs-lisp")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() && entry.file_name() != "cl-loaddefs.el" {
                fs::copy(entry.path(), fixture.directory.join(entry.file_name())).unwrap();
            }
        }
        fixture
    }

    pub fn path(&self) -> &Path {
        &self.directory
    }
}

impl Drop for SourceOnlyEmacsLisp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
