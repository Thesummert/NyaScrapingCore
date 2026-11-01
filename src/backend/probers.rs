use super::{GdbServerOptions, Operation};
use std::{path::Path, process::Command};

pub fn gdbserver_command(executable: &Path, options: &GdbServerOptions) -> Command {
    let mut command = Command::new(executable);
    command
        .args(["gdb", "--chip", &options.target, "--gdb-connection-string"])
        .arg(format!("127.0.0.1:{}", options.port));
    command
}

pub fn operation_command(
    executable: &Path,
    options: &GdbServerOptions,
    operation: &Operation,
) -> Command {
    let mut command = Command::new(executable);
    match operation {
        Operation::Flash(path) => {
            command
                .args(["download", "--chip", &options.target])
                .arg(path);
        }
        Operation::Erase { .. } => {
            command.args(["erase", "--chip", &options.target]);
        }
        Operation::Reset => {
            command.args(["reset", "--chip", &options.target]);
        }
    }
    command
}
