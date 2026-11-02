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

/// Adapter for pyOCD middleware.
#[derive(Clone, Copy, Debug, Default)]
pub struct PyOcd;

impl super::DebugBackend for PyOcd {
    fn tool(&self) -> crate::manager::Tool {
        crate::manager::Tool::PyOcd
    }

    fn gdbserver_command(
        &self,
        executable: &Path,
        options: &GdbServerOptions,
    ) -> super::Result<Command> {
        options.validate(self.tool())?;
        Ok(gdbserver_command(executable, options))
    }

    fn operation_command(
        &self,
        executable: &Path,
        options: &GdbServerOptions,
        operation: &Operation,
    ) -> super::Result<Command> {
        super::validate_operation(self.tool(), options, operation)?;
        Ok(operation_command(executable, options, operation))
    }
}
