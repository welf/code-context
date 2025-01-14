use std::path::{Path, PathBuf};

/// Handles module path resolution and manipulation
pub struct ModulePath {
    path: PathBuf,
}

impl ModulePath {
    /// Creates a new ModulePath from a Path
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// Checks if this is a valid Rust module path
    pub fn is_valid_module(&self) -> bool {
        self.path.extension().is_some_and(|ext| ext == "rs")
            && !self.path.to_str().is_some_and(|s| s.ends_with(".rs.txt"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_valid_module() {
        let valid_path = PathBuf::from("src/foo/bar.rs");
        let invalid_path = PathBuf::from("src/foo/bar.txt");

        assert!(ModulePath::new(&valid_path).is_valid_module());
        assert!(!ModulePath::new(&invalid_path).is_valid_module());
    }
}
