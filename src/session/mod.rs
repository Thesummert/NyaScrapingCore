//! GDB client and server lifecycle management.

use std::{
    error::Error,
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn gdb_command(gdb: &str, elf: &Path, port: u16) -> Command {
    let mut command = Command::new(gdb);
    command
        .arg(elf)
        .arg("-ex")
        .arg(format!("target remote localhost:{port}"));
    command
}

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn run(mut server_command: Command, gdb: &str, elf: &Path, port: u16) -> Result<()> {
    // Detect a pre-existing listener before launching a server, so the readiness
    // probe cannot accidentally connect GDB to an unrelated process.
    let port_guard = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .map_err(|e| format!("GDB port {port} is already in use or unavailable: {e}"))?;
    drop(port_guard);
    let mut server = Server(server_command.spawn()?);
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = server.0.try_wait()? {
            return Err(format!("GDB server exited before accepting connections: {status}").into());
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(
                format!("GDB server did not listen on port {port} within 10 seconds").into(),
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
    let status = gdb_command(gdb, elf, port).status()?;
    if !status.success() {
        return Err(format!("GDB exited with {status}").into());
    }
    Ok(())
}
