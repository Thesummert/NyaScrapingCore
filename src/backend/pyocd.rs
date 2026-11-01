use super::{GdbServerOptions, Operation};
use std::{path::Path, process::Command};

pub fn gdbserver_command(executable: &Path, options: &GdbServerOptions) -> Command {
    let mut command = Command::new(executable);
    command
        .args(["gdbserver", "--target", &options.target, "--port"])
        .arg(options.port.to_string());
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
                .args(["flash", "--target", &options.target])
                .arg(path);
        }
        Operation::Erase { .. } => {
            command.args(["erase", "--target", &options.target, "--chip"]);
        }
        Operation::Reset => {
            command.args(["reset", "--target", &options.target]);
        }
    }
    command
}
