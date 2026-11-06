//! Backend-independent GDB server configuration and process ownership.

use crate::{
    backend::{self, DebugBackend, GdbServerOptions, Result},
    manager::{Manager, Tool},
};
use std::{
    io,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

/// Reusable server configuration. Creating it does not start a process.
pub struct GdbServer<'a> {
    backend: &'a dyn DebugBackend,
    executable: PathBuf,
    options: GdbServerOptions,
}

impl GdbServer<'static> {
    pub fn new(
        tool: Tool,
        executable: impl Into<PathBuf>,
        options: GdbServerOptions,
    ) -> Result<Self> {
        Self::with_backend(backend::backend(tool), executable, options)
    }

    /// Use the executable selected by the debugger version manager.
    pub fn resolve(manager: &Manager, tool: Tool, options: GdbServerOptions) -> Result<Self> {
        options.validate(tool)?;
        Self::new(tool, manager.resolve(tool)?, options)
    }
}

impl<'a> GdbServer<'a> {
    /// Accept a custom adapter, including one supplied by a library caller.
    pub fn with_backend(
        backend: &'a dyn DebugBackend,
        executable: impl Into<PathBuf>,
        options: GdbServerOptions,
    ) -> Result<Self> {
        options.validate(backend.tool())?;
        Ok(Self {
            backend,
            executable: executable.into(),
            options,
        })
    }

    pub fn tool(&self) -> Tool {
        self.backend.tool()
    }
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub fn options(&self) -> &GdbServerOptions {
        &self.options
    }
    pub fn address(&self) -> SocketAddr {
        address(self.options.port)
    }

    /// Build a command for inspection or caller-controlled stdio and execution.
    pub fn command(&self) -> Result<Command> {
        self.backend
            .gdbserver_command(&self.executable, &self.options)
    }

    /// Start the server and wait for its TCP endpoint. On failure the child is reaped.
    pub fn start(&self, timeout: Duration) -> Result<RunningGdbServer> {
        RunningGdbServer::start(self.command()?, self.options.port, timeout)
    }
}

fn address(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// Owns the server process. Dropping this handle terminates and reaps the child.
/// Readiness means the local port is bound; it does not verify target hardware.
/// The endpoint check cannot eliminate races with other processes binding the port.
#[derive(Debug)]
pub struct RunningGdbServer {
    child: Child,
    address: SocketAddr,
}

impl RunningGdbServer {
    /// Start a prepared command, preserving caller-specified environment and stdio.
    pub fn start(mut command: Command, port: u16, timeout: Duration) -> Result<Self> {
        if port == 0 {
            return Err("GDB port must be between 1 and 65535".into());
        }
        if timeout.is_zero() {
            return Err("GDB server startup timeout must be greater than zero".into());
        }
        let address = address(port);
        let guard = TcpListener::bind(address)
            .map_err(|e| format!("GDB port {port} is already in use or unavailable: {e}"))?;
        drop(guard);
        let mut server = Self {
            child: command.spawn()?,
            address,
        };
        server.wait_ready(timeout)?;
        Ok(server)
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(
                    format!("GDB server exited before accepting connections: {status}").into(),
                );
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(format!(
                    "GDB server did not listen on {} within {timeout:?}",
                    self.address
                )
                .into());
            }
            if matches!(TcpListener::bind(self.address), Err(ref error) if error.kind() == io::ErrorKind::AddrInUse)
            {
                // Do not report success for a process that exited during the probe.
                if let Some(status) = self.child.try_wait()? {
                    return Err(format!("GDB server exited during startup: {status}").into());
                }
                return Ok(());
            }
            thread::sleep(
                timeout
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_millis(25)),
            );
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    /// Stop and reap the child. Repeated calls return its previously recorded status.
    pub fn stop(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.child.try_wait()? {
            return Ok(status);
        }
        if let Err(error) = self.child.kill() {
            // The process may have exited between try_wait and kill.
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            return Err(error);
        }
        self.child.wait()
    }
}

impl Drop for RunningGdbServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    fn options(tool: Tool) -> GdbServerOptions {
        GdbServerOptions {
            target: "chip or target config".into(),
            interface: (tool == Tool::OpenOcd).then(|| "interface config".into()),
            transport: None,
            port: 3333,
        }
    }

    #[test]
    fn configurations_use_each_backend_without_spawning() {
        for tool in Tool::ALL {
            let server = GdbServer::new(tool, "path with spaces/debugger", options(tool)).unwrap();
            let command = server.command().unwrap();
            assert_eq!(server.tool(), tool);
            assert_eq!(command.get_program(), server.executable().as_os_str());
            assert_eq!(server.address(), address(3333));
            assert!(command.get_args().any(|arg| arg == "chip or target config"));
        }
        let mut invalid = options(Tool::OpenOcd);
        invalid.interface = None;
        assert!(GdbServer::new(Tool::OpenOcd, "debugger", invalid).is_err());
    }

    fn unused_port() -> u16 {
        TcpListener::bind(address(0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn helper(mode: &str, port: u16) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "gdbserver::tests::server_process",
                "--nocapture",
            ])
            .env("NSC_TEST_SERVER_MODE", mode)
            .env("NSC_TEST_SERVER_ADDRESS", address(port).to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    // Run in a child test process so lifecycle tests need no installed debugger or shell.
    #[test]
    #[ignore = "subprocess helper for GDB server lifecycle tests"]
    fn server_process() {
        let Ok(mode) = std::env::var("NSC_TEST_SERVER_MODE") else {
            return;
        };
        if mode == "exit" {
            std::process::exit(7);
        }
        let _listener = if mode == "listen" || mode == "single" {
            Some(TcpListener::bind(std::env::var("NSC_TEST_SERVER_ADDRESS").unwrap()).unwrap())
        } else {
            None
        };
        if mode == "single" {
            let (mut stream, _) = _listener.unwrap().accept().unwrap();
            use std::io::Write;
            stream.write_all(b"first client").unwrap();
            return;
        }
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn readiness_does_not_consume_the_first_client() {
        use std::io::Read;
        let port = unused_port();
        let _server =
            RunningGdbServer::start(helper("single", port), port, Duration::from_secs(5)).unwrap();
        let mut stream = TcpStream::connect(address(port)).unwrap();
        let mut message = String::new();
        stream.read_to_string(&mut message).unwrap();
        assert_eq!(message, "first client");
    }

    #[test]
    fn starts_queries_and_stops_a_server() {
        let port = unused_port();
        let mut server =
            RunningGdbServer::start(helper("listen", port), port, Duration::from_secs(5)).unwrap();
        assert_eq!(server.address(), address(port));
        assert!(server.id() > 0);
        assert!(server.try_wait().unwrap().is_none());
        let status = server.stop().unwrap();
        assert_eq!(server.stop().unwrap(), status);
        assert_eq!(server.wait().unwrap(), status);
    }

    #[test]
    fn drop_reaps_the_child() {
        let port = unused_port();
        let server =
            RunningGdbServer::start(helper("listen", port), port, Duration::from_secs(5)).unwrap();
        drop(server);
        // Once Drop returns, the listener owned by the child has been closed.
        assert!(TcpStream::connect_timeout(&address(port), Duration::from_millis(100)).is_err());
    }

    #[test]
    fn rejects_occupied_port_before_spawning() {
        let listener = TcpListener::bind(address(0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        // A nonexistent executable proves that the port check happens before spawn.
        let error = RunningGdbServer::start(
            Command::new("nonexistent-gdbserver-for-test"),
            port,
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("already in use or unavailable"));
    }

    #[test]
    fn reports_early_exit_and_startup_timeout() {
        let port = unused_port();
        let error = RunningGdbServer::start(helper("exit", port), port, Duration::from_secs(5))
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("exited before accepting connections"));
        let port = unused_port();
        let error =
            RunningGdbServer::start(helper("sleep", port), port, Duration::from_millis(100))
                .unwrap_err();
        assert!(error.to_string().contains("did not listen"));
    }

    #[test]
    fn rejects_invalid_start_parameters() {
        let error =
            RunningGdbServer::start(Command::new("unused"), 0, Duration::from_secs(1)).unwrap_err();
        assert!(error.to_string().contains("port"));
        let error =
            RunningGdbServer::start(Command::new("unused"), 3333, Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("timeout"));
    }
}
