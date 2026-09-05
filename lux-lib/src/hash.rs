use bytes::Bytes;
use camino::Utf8Path;
use ignore::gitignore::Gitignore;
use nix_nar::{Encoder, FileSystem, FileSystemMetadata, NativeFileSystem};
use ssri::{Algorithm, Integrity, IntegrityOpts};
use std::fs::File;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

pub trait HasIntegrity {
    fn hash(&self) -> impl Future<Output = io::Result<Integrity>> + Send;
}

impl HasIntegrity for PathBuf {
    #[tracing::instrument(level = "trace", skip_all)]
    async fn hash(&self) -> io::Result<Integrity> {
        // Canonicalising ensures a symlinked directory
        // is hashed by its contents rather than as a symlink.
        let path = std::fs::canonicalize(self)?;
        tokio::task::spawn_blocking(move || {
            let mut integrity_opts = IntegrityOpts::new().algorithm(Algorithm::Sha256);
            if path.is_dir() {
                // NOTE: To ensure our source hashes are compatible with Nix,
                // we encode the path to the Nix Archive (NAR) format.
                let filesystem = VcsExcludingFileSystem::new(&path);
                let mut enc =
                    Encoder::new_with_filesystem(path, filesystem).map_err(io::Error::other)?;
                io::copy(&mut enc, &mut integrity_opts)?;
            } else if path.is_file() {
                hash_file(&path, &mut integrity_opts)?;
            }
            Ok(integrity_opts.result())
        })
        .await
        .map_err(io::Error::other)?
    }
}

impl HasIntegrity for Path {
    async fn hash(&self) -> io::Result<Integrity> {
        let path_buf: PathBuf = self.into();
        path_buf.hash().await
    }
}

impl HasIntegrity for Bytes {
    #[tracing::instrument(level = "trace", skip_all)]
    async fn hash(&self) -> io::Result<Integrity> {
        let bytes = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut integrity_opts = IntegrityOpts::new().algorithm(Algorithm::Sha256);
            integrity_opts.input(&bytes);
            Ok(integrity_opts.result())
        })
        .await
        .map_err(io::Error::other)?
    }
}

fn hash_file(path: &Path, integrity_opts: &mut IntegrityOpts) -> io::Result<()> {
    let mut file = File::open(path)?;
    io::copy(&mut file, integrity_opts)?;
    Ok(())
}

/// A [`FileSystem`] that excludes VCS directories (e.g. `.git`, `.jj`) and
/// files ignored by the project's `.gitignore` from the encoded
/// NAR, as their contents are not deterministic and are not part of a
/// package's sources.
struct VcsExcludingFileSystem {
    gitignore: Gitignore,
}

impl VcsExcludingFileSystem {
    fn new(root: &Path) -> Self {
        let (gitignore, err) = Gitignore::new(root.join(".gitignore"));
        if let Some(err) = err {
            tracing::debug!(
                message =
                    format!("failed to parse `.gitignore` in {}: {err}", root.display()).as_str()
            );
        }
        Self { gitignore }
    }
}

fn is_vcs_dir(name: &str) -> bool {
    matches!(name, ".git" | ".jj" | ".hg" | "_darcs" | ".svn" | ".bzr")
}

impl FileSystem for VcsExcludingFileSystem {
    type File = std::fs::File;

    fn open(&self, path: &Utf8Path) -> io::Result<Self::File> {
        FileSystem::open(&NativeFileSystem {}, path)
    }

    fn read_dir(&self, path: &Utf8Path) -> io::Result<Vec<String>> {
        Ok(FileSystem::read_dir(&NativeFileSystem {}, path)?
            .into_iter()
            .filter(|name| !is_vcs_dir(name))
            .filter(|name| {
                let full = path.join(name);
                let is_dir = full.as_std_path().is_dir();
                !self
                    .gitignore
                    .matched(full.as_std_path(), is_dir)
                    .is_ignore()
            })
            .collect())
    }

    fn metadata(&self, path: &Utf8Path) -> io::Result<FileSystemMetadata> {
        FileSystem::metadata(&NativeFileSystem {}, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use std::{fs::write, process::Command};

    #[cfg(unix)]
    /// Compute nix-hash --sri --type sha256 .
    fn nix_hash(path: &Path) -> Integrity {
        let ssri_str = Command::new("nix-hash")
            .args(vec!["--sri", "--type", "sha256"])
            .arg(path)
            .output()
            .unwrap()
            .stdout;
        String::from_utf8_lossy(&ssri_str).parse().unwrap()
    }

    #[cfg(unix)]
    /// Compute nix-hash --sri --type sha256 --flat .
    fn nix_hash_file(path: &Path) -> Integrity {
        let ssri_str = Command::new("nix-hash")
            .args(vec!["--sri", "--type", "sha256", "--flat"])
            .arg(path)
            .output()
            .unwrap()
            .stdout;
        String::from_utf8_lossy(&ssri_str).parse().unwrap()
    }

    #[tokio::test]
    async fn test_hash_empty_dir() {
        let temp = assert_fs::TempDir::new().unwrap();
        let hash1 = temp.path().to_path_buf().hash().await.unwrap();
        let hash2 = temp.path().to_path_buf().hash().await.unwrap();
        assert_eq!(hash1, hash2);
        let nix_hash = nix_hash(temp.path());
        assert_eq!(hash1, nix_hash);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_hash_file() {
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test.txt");
        file.write_str("test content").unwrap();

        let hash = file.path().to_path_buf().hash().await.unwrap();
        let nix_hash = nix_hash_file(file.path());
        assert_eq!(hash, nix_hash);
    }

    #[tokio::test]
    async fn test_hash_dir_with_single_file() {
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test.txt");
        file.write_str("test content").unwrap();

        let hash1 = temp.path().to_path_buf().hash().await.unwrap();
        let hash2 = temp.path().to_path_buf().hash().await.unwrap();
        assert_eq!(hash1, hash2);

        #[cfg(unix)]
        {
            let nix_hash = nix_hash(temp.path());
            assert_eq!(hash1, nix_hash);
        }
    }

    #[tokio::test]
    async fn test_hash_multiple_files_different_creation_order() {
        let temp = assert_fs::TempDir::new().unwrap();

        write(temp.child("a.txt").path(), "content a").unwrap();
        write(temp.child("b.txt").path(), "content b").unwrap();
        write(temp.child("c.txt").path(), "content c").unwrap();
        let hash1 = temp.path().to_path_buf().hash().await.unwrap();

        let temp2 = assert_fs::TempDir::new().unwrap();
        write(temp2.child("c.txt").path(), "content c").unwrap();
        write(temp2.child("a.txt").path(), "content a").unwrap();
        write(temp2.child("b.txt").path(), "content b").unwrap();
        let hash2 = temp2.path().to_path_buf().hash().await.unwrap();

        assert_eq!(hash1, hash2);

        #[cfg(unix)]
        {
            let nix_hash = nix_hash(temp.path());
            assert_eq!(hash1, nix_hash);
        }
    }

    #[tokio::test]
    async fn test_hash_nested_directories_different_creation_order() {
        let temp = assert_fs::TempDir::new().unwrap();

        temp.child("a/b").create_dir_all().unwrap();
        temp.child("b").create_dir_all().unwrap();
        write(temp.child("a/b/file1.txt").path(), "content 1").unwrap();
        write(temp.child("a/file2.txt").path(), "content 2").unwrap();
        write(temp.child("b/file3.txt").path(), "content 3").unwrap();
        let hash1 = temp.path().to_path_buf().hash().await.unwrap();

        let temp2 = assert_fs::TempDir::new().unwrap();
        temp2.child("a/b").create_dir_all().unwrap();
        temp2.child("b").create_dir_all().unwrap();
        write(temp2.child("b/file3.txt").path(), "content 3").unwrap();
        write(temp2.child("a/file2.txt").path(), "content 2").unwrap();
        write(temp2.child("a/b/file1.txt").path(), "content 1").unwrap();
        let hash2 = temp2.path().to_path_buf().hash().await.unwrap();

        assert_eq!(hash1, hash2);
    }

    #[tokio::test]
    async fn test_hash_with_different_line_endings() {
        let temp = assert_fs::TempDir::new().unwrap();
        write(temp.child("unix.txt").path(), "line1\nline2\n").unwrap();
        let hash1 = temp.path().to_path_buf().hash().await.unwrap();

        let temp2 = assert_fs::TempDir::new().unwrap();
        write(temp2.child("windows.txt").path(), "line1\r\nline2\r\n").unwrap();
        let hash2 = temp2.path().to_path_buf().hash().await.unwrap();

        assert_ne!(hash1, hash2);
    }

    #[tokio::test]
    async fn test_hash_ignores_vcs_directories() {
        let temp = assert_fs::TempDir::new().unwrap();
        write(temp.child("src.lua").path(), "return true").unwrap();
        temp.child(".git").create_dir_all().unwrap();
        write(temp.child(".git/config").path(), "nondeterministic").unwrap();
        temp.child(".jj").create_dir_all().unwrap();
        write(temp.child(".jj/repo").path(), "nondeterministic").unwrap();

        let hash_with_vcs = temp.path().to_path_buf().hash().await.unwrap();

        let temp2 = assert_fs::TempDir::new().unwrap();
        write(temp2.child("src.lua").path(), "return true").unwrap();

        let hash_without_vcs = temp2.path().to_path_buf().hash().await.unwrap();

        assert_eq!(hash_with_vcs, hash_without_vcs);
    }

    #[tokio::test]
    async fn test_hash_ignores_gitignored_but_not_other_ignored_files() {
        let temp = assert_fs::TempDir::new().unwrap();
        write(temp.child("src.lua").path(), "return true").unwrap();
        temp.child(".gitignore").write_str(".foo/\n").unwrap();
        temp.child(".ignore").write_str("target/\n").unwrap();
        temp.child(".foo").create_dir_all().unwrap();
        write(temp.child(".foo/env").path(), "bar").unwrap();
        temp.child("target").create_dir_all().unwrap();
        write(temp.child("target/build.o").path(), "blob").unwrap();

        let filtered = temp.path().to_path_buf().hash().await.unwrap();

        let expected_tree = assert_fs::TempDir::new().unwrap();
        write(expected_tree.child("src.lua").path(), "return true").unwrap();
        expected_tree
            .child(".gitignore")
            .write_str(".foo/\n")
            .unwrap();
        expected_tree
            .child(".ignore")
            .write_str("target/\n")
            .unwrap();
        expected_tree.child("target").create_dir_all().unwrap();
        write(expected_tree.child("target/build.o").path(), "blob").unwrap();
        let expected = expected_tree.path().to_path_buf().hash().await.unwrap();

        assert_eq!(filtered, expected);
    }

    #[tokio::test]
    async fn test_hash_matches_plain_nar_without_vcs_dirs() {
        let temp = assert_fs::TempDir::new().unwrap();
        write(temp.child("src.lua").path(), "return true").unwrap();
        temp.child("lua").create_dir_all().unwrap();
        write(temp.child("lua/foo.lua").path(), "return 1").unwrap();

        let filtered = temp.path().to_path_buf().hash().await.unwrap();

        let mut opts = IntegrityOpts::new().algorithm(Algorithm::Sha256);
        let mut enc = Encoder::new(temp.path()).unwrap();
        io::copy(&mut enc, &mut opts).unwrap();
        let plain = opts.result();

        assert_eq!(filtered, plain);
    }

    #[tokio::test]
    async fn test_hash_with_symlinks() {
        let temp = assert_fs::TempDir::new().unwrap();

        write(temp.child("target.txt").path(), "content").unwrap();

        #[cfg(target_family = "unix")]
        std::os::unix::fs::symlink(
            temp.child("target.txt").path(),
            temp.child("link.txt").path(),
        )
        .unwrap();
        #[cfg(target_family = "windows")]
        std::os::windows::fs::symlink_file(
            temp.child("target.txt").path(),
            temp.child("link.txt").path(),
        )
        .unwrap();

        let hash1 = temp.path().to_path_buf().hash().await.unwrap();

        let temp2 = assert_fs::TempDir::new().unwrap();
        write(temp2.child("target.txt").path(), "content").unwrap();
        let hash2 = temp2.path().to_path_buf().hash().await.unwrap();

        assert_ne!(hash1, hash2);

        #[cfg(unix)]
        {
            let nix_hash = nix_hash(temp.path());
            assert_eq!(hash1, nix_hash);
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_hash_dereferences_symlinked_directory() {
        let outer = assert_fs::TempDir::new().unwrap();
        let real = assert_fs::TempDir::new().unwrap();
        write(real.child("src.lua").path(), "return true").unwrap();

        let link = outer.path().join("link");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();

        let hash_via_link = link.hash().await.unwrap();
        let hash_real = real.path().to_path_buf().hash().await.unwrap();

        assert_eq!(hash_via_link, hash_real);
    }
}
