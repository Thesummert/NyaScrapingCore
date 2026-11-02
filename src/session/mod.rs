//! GDB client and server lifecycle management.

use crate::gdbserver::RunningGdbServer;
use std::{error::Error, path::Path, process::Command, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn gdb_command(gdb: &str, elf: &Path, port: u16) -> Command {
    let mut command = Command::new(gdb);
    command
        .arg(elf)
        .arg("-ex")
        .arg(format!("target remote localhost:{port}"));
    command
}

pub fn run(server_command: Command, gdb: &str, elf: &Path, port: u16) -> Result<()> {
    let _server = RunningGdbServer::start(server_command, port, Duration::from_secs(10))?;
    let status = gdb_command(gdb, elf, port).status()?;
    if !status.success() {
        return Err(format!("GDB exited with {status}").into());
    }
    Ok(())
}
