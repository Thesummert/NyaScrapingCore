# 调试器版本管理

使用 `cargo run -- <命令>` 或编译后的 `nsc <命令>`：

```text
list [openocd|pyocd|probe-rs]   列出已安装版本及 PATH 中的程序
install <工具> [版本]            安装指定版本；省略版本时安装最新版
uninstall <工具> <版本>          删除本工具管理的指定版本（也可用 remove）
add-local <工具> <名称> <目录>   注册已有的本地调试工具
remove-local <工具> <名称>       移除本地注册，不删除原文件
use <工具> <版本|system|local:名称> 保存选择；system 选择 PATH 中的第一个程序
which <工具>                     输出当前选择的可执行文件路径
paths                            显示配置文件及安装目录
gdbserver <工具> --target <目标> [--interface <文件>] [--port <端口>] [--dry-run]
flash <工具> --target <目标> --file <固件> [--interface <文件>] [--dry-run]
erase <工具> --target <目标> [--interface <文件>] [--bank <编号>] [--dry-run]
reset <工具> --target <目标> [--interface <文件>] [--dry-run]
debug <工具> --target <目标> --file <ELF> [--interface <文件>] [--port <端口>] [--gdb <程序>] [--dry-run]
```

例如：

```sh
nsc install openocd
nsc install pyocd 0.36.0
nsc install probe-rs 0.31.0
nsc use openocd 0.12.0-7
nsc use pyocd system
nsc add-local openocd custom "/path/to/openocd"
nsc use openocd local:custom
nsc list
nsc uninstall openocd 0.12.0-7
nsc gdbserver openocd --interface interface/stlink.cfg --target target/stm32f4x.cfg
nsc gdbserver pyocd --target stm32f407vg --port 3333
nsc gdbserver probe-rs --target STM32F407VG --port 3333
nsc flash pyocd --target stm32f407vg --file build/app.elf
nsc erase openocd --interface interface/stlink.cfg --target target/stm32f4x.cfg --bank 0
nsc reset probe-rs --target STM32F407VG
nsc debug probe-rs --target STM32F407VG --file build/app.elf
```

`gdbserver` 对三种工具使用同一命令格式，启动前会根据 `use` 保存的选择寻找可执行文件；未选择时使用 PATH。`--port` 默认为 3333，允许 1 到 65535。OpenOCD 必须提供 `--interface`，其 `--target` 是目标配置文件；pyOCD 和 probe-rs 的 `--target` 是芯片名，不接受 `--interface`。`--dry-run` 只打印要执行的进程及参数。启动后，服务在前台运行，可从另一个终端用 GDB 连接 `localhost:3333`（或指定端口）。库调用方可使用 `backend::gdbserver_command` 和 `GdbServerOptions` 构造对应的 `std::process::Command`。

`flash` 烧录并校验固件；OpenOCD 使用其 `program ... verify reset exit` 流程，固件应为 ELF、HEX 或 S19，其他格式按各工具支持情况使用。`erase` 对 pyOCD 和 probe-rs 执行芯片擦除；OpenOCD 必须用 `--bank` 指定要擦除的 flash bank 编号，可用原生 `flash banks` 查看。擦除有目标芯片差异，执行前建议先用 `--dry-run` 核对命令。`reset` 复位目标。`debug` 启动 GDB 服务、等待端口可连接、打开 `arm-none-eabi-gdb` 并自动执行 `target remote localhost:<端口>`，GDB 退出时关闭服务；可用 `--gdb` 指定其他 GDB 程序。库调用方可使用 `backend::operation_command` 和 `Operation` 构造烧录、擦除及复位命令，并使用 `session::run` 管理调试会话。

省略版本时会查询对应软件源的最新稳定版，OpenOCD 选择包含当前平台二进制文件的最新 xPack 正式发布。下载和构建进度由 curl、pip 或 Cargo 显示，具体形式取决于工具和终端；probe-rs 从源码构建，Cargo 的进度包含编译阶段。

`probers` 是 `probe-rs` 的别名。未设置选择时，`which` 使用 PATH 中的第一个匹配项；已选择的托管版本不存在时会报错。配置只影响本程序的解析，不会修改系统 PATH。`list` 检测 PATH 中所有同名程序并调用 `--version`；运行它们时请确保 PATH 中的程序可信。

本地注册支持 `openocd`、`pyocd` 和 `probe-rs`。目录可以是可执行文件所在目录，或包含 `bin` / `Scripts` 子目录的安装目录；名称由英文字母、数字、`.`、`-` 和 `_` 组成。同一工具下名称不能重复。注册保存绝对路径，不复制文件；移动或删除原程序后需要重新注册。`list` 会标记丢失的本地程序。使用 `remove-local <工具> <名称>` 移除注册时，如果它正被选中，会清除选择并回退到 PATH；原目录及文件不会被删除。本地注册保存在配置文件旁的 `config.local.json` 中。

卸载需要明确指定版本，只删除该版本的托管目录，其他版本及系统 PATH 中的程序不受影响。如果卸载的是当前选中版本，会清除该工具的选择，之后 `which` 将尝试使用 PATH 中的程序。

配置默认存于 Windows `%APPDATA%/NyaScrapingCore/config`、macOS `~/Library/Application Support/NyaScrapingCore/config`，或 Linux `$XDG_CONFIG_HOME/NyaScrapingCore/config`（默认 `~/.config`）。安装文件分别存于 Windows `%LOCALAPPDATA%/NyaScrapingCore/debuggers`、macOS `~/Library/Application Support/NyaScrapingCore/debuggers`，或 Linux `$XDG_DATA_HOME/NyaScrapingCore/debuggers`（默认 `~/.local/share`）。

OpenOCD 使用 [xPack 二进制发布包](https://xpack.github.io/openocd/install/)，版本需填写其发布版本，如 `0.12.0-7`；需要 `curl` 和支持该压缩格式的 `tar`。pyOCD 使用 [Python 虚拟环境和 pip](https://pyocd.io/docs/installing)，需要 Python 和 venv/pip。probe-rs 使用 [cargo install](https://probe.rs/docs/getting-started/installation/)，需要 Rust 工具链及平台构建依赖，编译可能耗时。安装均需要访问相应的软件源；版本和平台必须有对应的发行包。安装失败会清理本次创建的版本目录，不会覆盖已有目录。

## GDB server 抽象层（Rust）

`backend::DebugBackend` 提供统一的 `tool()`、`gdbserver_command()` 和
`operation_command()` 接口。内置实现为 `openocd::OpenOcd`、`pyocd::PyOcd`、
`probers::ProbeRs`，可通过 `backend::backend(tool)` 获取 `&dyn DebugBackend`。
原有 `backend::gdbserver_command` 和 `operation_command` 保留，内部委托给对应实现。

`gdbserver::GdbServer` 统一配置和启动入口，`RunningGdbServer` 管理进程：

```rust,no_run
use nya_scraping_core::{
    backend::GdbServerOptions,
    gdbserver::GdbServer,
    manager::{Manager, Tool},
};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manager = Manager::new()?;
    let server = GdbServer::resolve(&manager, Tool::PyOcd, GdbServerOptions {
        target: "stm32f407vg".into(),
        interface: None,
        port: 3333,
    })?;
    println!("{:?}", server.command()?); // 仅构造命令，不启动进程
    let mut running = server.start(Duration::from_secs(10))?;
    println!("PID: {}, GDB 地址: {}", running.id(), running.address());
    // 调用方在这里连接 GDB、执行调试；try_wait() 可查询进程是否退出。
    running.stop()?;
    Ok(())
}
```

也可用 `GdbServer::new(tool, executable, options)` 显式指定程序，或用
`GdbServer::with_backend` 注入自定义适配器。需要设置日志输出、环境变量等时，先修改
`server.command()?` 返回的 `Command`，再交给 `RunningGdbServer::start`。

`start` 检查本机端口、启动进程并等待 TCP 连接成功；端口占用、启动失败、进程提前退出
或等待超时均返回错误。就绪检查只代表 TCP 端口可连接，不验证芯片状态或 GDB 协议；
端口检查和子进程绑定之间仍存在竞争窗口。服务句柄支持 `id`、`address`、`try_wait`、
`wait` 和可重复调用的 `stop`；句柄离开作用域时终止并回收直接子进程。
`session::run` 已复用此生命周期实现，GDB 客户端退出或启动失败时会清理服务。

## OpenOCD J-Link 传输方式

内置 `interface/jlink.cfg` 在连接、烧录、擦除和复位时默认使用 SWD。
仅支持 SWD 的 J-Link OB 探针会拒绝 OpenOCD 自动选择的 JTAG。
使用 JTAG 接线时可显式指定：

```sh
nsc gdbserver openocd --interface interface/jlink.cfg --transport jtag --target target/stm32h7x.cfg
```

`--transport` 仅用于 OpenOCD，可选 `swd`、`jtag`、`hla_swd`、`hla_jtag`。
传输方式在加载 Probe 配置后、加载目标配置前设置。
自定义 Probe 配置保留自己的默认方式，也可以通过该选项覆盖。
