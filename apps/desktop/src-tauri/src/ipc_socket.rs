use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
    time::Duration,
};

pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    _lock: File,
}

impl Server {
    pub fn bind(dir: &Path) -> io::Result<Self> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(dir.join("fluxcast.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(io::ErrorKind::AddrInUse.into()),
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        }
        let path = dir.join("fluxcast.sock");
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.file_type().is_socket() {
                return Err(io::Error::other("fluxcast.sock existe y no es un socket"));
            }
            fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            path,
            _lock: lock,
        })
    }

    pub fn serve(&self, toggle: impl Fn() -> bool) {
        for mut client in self.listener.incoming().flatten() {
            let _ = client.set_read_timeout(Some(Duration::from_millis(250)));
            let mut cmd = [0; 7];
            let ok = client.read_exact(&mut cmd).is_ok() && &cmd == b"toggle\n" && toggle();
            let _ = client.write_all(if ok { b"OK\n" } else { b"ERR\n" });
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn singleton_recovers_stale_socket_and_preserves_unrelated_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fluxcast.sock");
        drop(UnixListener::bind(&path).unwrap());
        let server = Server::bind(temp.path()).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            matches!(Server::bind(temp.path()), Err(e) if e.kind() == io::ErrorKind::AddrInUse)
        );
        drop(server);
        fs::write(&path, "conservar").unwrap();
        assert!(Server::bind(temp.path()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "conservar");
    }

    #[test]
    fn client_toggle_roundtrip_acknowledges_success_and_rejection() {
        let temp = tempfile::tempdir().unwrap();
        let server = Server::bind(temp.path()).unwrap();
        let allow = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&allow);
        std::thread::spawn(move || {
            server.serve(|| flag.load(Ordering::SeqCst));
        });
        assert!(client::toggle(temp.path()).is_ok());
        allow.store(false, Ordering::SeqCst);
        assert!(client::toggle(temp.path()).is_err());
    }
}
