//! A common GDB server command for the supported debugger tools.

use crate::manager::Tool;
use std::{
    error::Error,
    path::{Path, PathBuf},
    process::Command,
};

pub mod openocd;
pub mod probers;
pub mod pyocd;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Object-safe interface shared by all debugger middleware adapters.
/// Implementations validate options before constructing a command and never spawn it.
pub trait DebugBackend: Send + Sync {
    fn tool(&self) -> Tool;
    fn gdbserver_command(&self, executable: &Path, options: &GdbServerOptions) -> Result<Command>;
    fn operation_command(
        &self,
        executable: &Path,
        options: &GdbServerOptions,
        operation: &Operation,
    ) -> Result<Command>;
}

/// Select a built-in adapter without allocating or starting an external tool.
pub fn backend(tool: Tool) -> &'static dyn DebugBackend {
    match tool {
        Tool::OpenOcd => &openocd::OpenOcd,
        Tool::PyOcd => &pyocd::PyOcd,
        Tool::ProbeRs => &probers::ProbeRs,
    }
}

/// `target` is an OpenOCD target configuration file or a pyOCD/probe-rs chip name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GdbServerOptions {
    pub target: String,
    pub interface: Option<String>,
    /// OpenOCD transport override, selected after the probe and before the target.
    pub transport: Option<String>,
    pub port: u16,
}

impl GdbServerOptions {
    pub fn validate(&self, tool: Tool) -> Result<()> {
        if self.target.trim().is_empty() || self.target.starts_with('-') {
            return Err("target must be a nonempty target name or configuration file".into());
        }
        if self.port == 0 {
            return Err("GDB port must be between 1 and 65535".into());
        }
        if let Some(transport) = &self.transport {
            if tool != Tool::OpenOcd {
                return Err("--transport is only supported by OpenOCD".into());
            }
            if !matches!(transport.as_str(), "swd" | "jtag" | "hla_swd" | "hla_jtag") {
                return Err("invalid OpenOCD transport; use swd, jtag, hla_swd or hla_jtag".into());
            }
        }
        match (tool, self.interface.as_deref()) {
            (Tool::OpenOcd, Some(value)) if !value.trim().is_empty() && !value.starts_with('-') => {
            }
            (Tool::OpenOcd, _) => return Err("OpenOCD requires --interface".into()),
            (_, Some(_)) => return Err("--interface is only supported by OpenOCD".into()),
            _ => {}
        }
        Ok(())
    }
}

/// Build a process command without starting it, allowing callers to manage its lifecycle.
pub fn gdbserver_command(
    tool: Tool,
    executable: &Path,
    options: &GdbServerOptions,
) -> Result<Command> {
    backend(tool).gdbserver_command(executable, options)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Flash(PathBuf),
    Erase { bank: Option<u32> },
    Reset,
}

/// Construct a one-shot maintenance command for any supported debugger.
pub fn operation_command(
    tool: Tool,
    executable: &Path,
    options: &GdbServerOptions,
    operation: &Operation,
) -> Result<Command> {
    backend(tool).operation_command(executable, options, operation)
}

fn validate_operation(tool: Tool, options: &GdbServerOptions, operation: &Operation) -> Result<()> {
    options.validate(tool)?;
    match operation {
        Operation::Flash(path) if !path.is_file() => {
            return Err(format!("firmware file does not exist: {}", path.display()).into())
        }
        Operation::Erase { bank: Some(_) } if tool != Tool::OpenOcd => {
            return Err("--bank is only supported by OpenOCD".into())
        }
        Operation::Erase { bank: None } if tool == Tool::OpenOcd => {
            return Err("OpenOCD erase requires --bank".into())
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapters_validate_direct_trait_calls() {
        let executable = Path::new("debugger");
        for tool in Tool::ALL {
            let adapter: &dyn DebugBackend = backend(tool);
            assert_eq!(adapter.tool(), tool);
            let options = GdbServerOptions {
                target: "chip".into(),
                interface: (tool == Tool::OpenOcd).then(|| "interface.cfg".into()),
                transport: None,
                port: 3333,
            };
            let invalid_erase = Operation::Erase {
                bank: if tool == Tool::OpenOcd { None } else { Some(0) },
            };
            assert!(adapter
                .operation_command(executable, &options, &invalid_erase)
                .is_err());
            let mut invalid = options;
            invalid.port = 0;
            assert!(adapter.gdbserver_command(executable, &invalid).is_err());
        }
    }

    #[test]
    fn builds_all_three_gdb_servers() {
        let exe = Path::new("debugger");
        let mut options = GdbServerOptions {
            target: "target/stm32f4x.cfg".into(),
            interface: Some("interface/stlink.cfg".into()),
            transport: None,
            port: 4444,
        };
        let command = gdbserver_command(Tool::OpenOcd, exe, &options).unwrap();
        assert_eq!(
            command
                .get_args()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "-f",
                "interface/stlink.cfg",
                "-f",
                "target/stm32f4x.cfg",
                "-c",
                "gdb_port 4444"
            ]
        );

        options.interface = None;
        options.target = "STM32F407VG".into();
        let command = gdbserver_command(Tool::PyOcd, exe, &options).unwrap();
        assert_eq!(
            command
                .get_args()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "gdbserver",
                "--target",
                "STM32F407VG",
                "--port",
                "4444",
                "--connect",
                "attach"
            ]
        );
        let command = gdbserver_command(Tool::ProbeRs, exe, &options).unwrap();
        assert_eq!(
            command
                .get_args()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "gdb",
                "--chip",
                "STM32F407VG",
                "--gdb-connection-string",
                "127.0.0.1:4444"
            ]
        );
    }

    #[test]
    fn h723_jlink_uses_the_known_core_for_block_memory_transfers() {
        let mut options = GdbServerOptions {
            target: "stm32h723xx".into(),
            interface: None,
            transport: None,
            port: 3333,
        };
        for target in ["stm32h723xx", "STM32H723XX", "cortex_m", "stm32f407vg"] {
            options.target = target.into();
            let command = gdbserver_command(Tool::PyOcd, Path::new("pyocd"), &options).unwrap();
            assert_eq!(
                command
                    .get_args()
                    .any(|arg| arg == "jlink.device=Cortex-M7"),
                target.eq_ignore_ascii_case("stm32h723xx")
            );
        }
    }

    #[test]
    fn rejects_invalid_tool_specific_options() {
        let exe = Path::new("debugger");
        let mut options = GdbServerOptions {
            target: "chip".into(),
            interface: None,
            transport: None,
            port: 3333,
        };
        assert!(gdbserver_command(Tool::OpenOcd, exe, &options).is_err());
        options.interface = Some("stlink.cfg".into());
        assert!(gdbserver_command(Tool::PyOcd, exe, &options).is_err());
        options.interface = None;
        options.port = 0;
        assert!(gdbserver_command(Tool::ProbeRs, exe, &options).is_err());
    }

    #[test]
    fn jlink_transport_is_selected_before_target_for_connections_and_maintenance() {
        let exe = Path::new("openocd");
        let mut options = GdbServerOptions {
            target: "target/stm32h7x.cfg".into(),
            interface: Some("interface/jlink.cfg".into()),
            transport: None,
            port: 3333,
        };
        for interface in [
            "interface/jlink.cfg",
            "/opt/openocd/scripts/interface/jlink.cfg",
            "C:\\OpenOCD\\scripts\\interface\\jlink.cfg",
        ] {
            options.interface = Some(interface.into());
            for transport in [None, Some("jtag")] {
                options.transport = transport.map(str::to_owned);
                let commands = [
                    gdbserver_command(Tool::OpenOcd, exe, &options).unwrap(),
                    operation_command(Tool::OpenOcd, exe, &options, &Operation::Reset).unwrap(),
                    operation_command(
                        Tool::OpenOcd,
                        exe,
                        &options,
                        &Operation::Flash("Cargo.toml".into()),
                    )
                    .unwrap(),
                ];
                for command in commands {
                    let args: Vec<_> = command.get_args().map(|v| v.to_str().unwrap()).collect();
                    assert_eq!(
                        &args[..6],
                        [
                            "-f",
                            interface,
                            "-c",
                            &format!("transport select {}", transport.unwrap_or("swd")),
                            "-f",
                            "target/stm32h7x.cfg",
                        ]
                    );
                }
            }
        }
        options.interface = Some("/custom/jlink.cfg".into());
        options.transport = None;
        let command = gdbserver_command(Tool::OpenOcd, exe, &options).unwrap();
        assert!(!command
            .get_args()
            .any(|arg| arg.to_string_lossy().contains("transport select")));
        for transport in ["swd; shutdown", "", "unknown"] {
            options.transport = Some(transport.into());
            assert!(gdbserver_command(Tool::OpenOcd, exe, &options).is_err());
        }
        options.interface = None;
        options.transport = Some("swd".into());
        for tool in [Tool::PyOcd, Tool::ProbeRs] {
            assert!(gdbserver_command(tool, exe, &options).is_err());
        }
    }

    #[test]
    fn builds_maintenance_commands() {
        let exe = Path::new("debugger");
        let mut options = GdbServerOptions {
            target: "target/stm32f4x.cfg".into(),
            interface: Some("interface/stlink.cfg".into()),
            transport: None,
            port: 3333,
        };
        let reset = operation_command(Tool::OpenOcd, exe, &options, &Operation::Reset).unwrap();
        assert_eq!(
            reset.get_args().last().unwrap(),
            "init; reset run; shutdown"
        );
        let erase = operation_command(
            Tool::OpenOcd,
            exe,
            &options,
            &Operation::Erase { bank: Some(1) },
        )
        .unwrap();
        assert_eq!(
            erase.get_args().last().unwrap(),
            "init; reset halt; flash erase_sector 1 0 last; shutdown"
        );
        assert!(operation_command(
            Tool::OpenOcd,
            exe,
            &options,
            &Operation::Erase { bank: None }
        )
        .is_err());

        options.interface = None;
        options.target = "STM32F407VG".into();
        let py = operation_command(Tool::PyOcd, exe, &options, &Operation::Erase { bank: None })
            .unwrap();
        assert_eq!(
            py.get_args()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["erase", "--target", "STM32F407VG", "--chip"]
        );
        let probe = operation_command(Tool::ProbeRs, exe, &options, &Operation::Reset).unwrap();
        assert_eq!(
            probe
                .get_args()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["reset", "--chip", "STM32F407VG"]
        );
        assert!(operation_command(
            Tool::ProbeRs,
            exe,
            &options,
            &Operation::Erase { bank: Some(0) }
        )
        .is_err());

        let firmware = PathBuf::from("Cargo.toml");
        let flash = Operation::Flash(firmware.clone());
        let py = operation_command(Tool::PyOcd, exe, &options, &flash).unwrap();
        assert_eq!(
            py.get_args()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["flash", "--target", "STM32F407VG", "Cargo.toml"]
        );
        let probe = operation_command(Tool::ProbeRs, exe, &options, &flash).unwrap();
        assert_eq!(
            probe
                .get_args()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["download", "--chip", "STM32F407VG", "Cargo.toml"]
        );
        options.interface = Some("interface/stlink.cfg".into());
        options.target = "target/stm32f4x.cfg".into();
        let openocd = operation_command(Tool::OpenOcd, exe, &options, &flash).unwrap();
        assert_eq!(
            openocd.get_args().last().unwrap(),
            "program {Cargo.toml} verify reset exit"
        );
    }
}
