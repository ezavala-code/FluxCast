use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

pub fn toggle(dir: &Path) -> io::Result<()> {
    let mut stream = UnixStream::connect(dir.join("fluxcast.sock"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"toggle\n")?;
    let mut ack = [0; 3];
    stream.read_exact(&mut ack)?;
    if &ack != b"OK\n" {
        return Err(io::Error::other("El launcher rechazó la solicitud"));
    }
    Ok(())
}
