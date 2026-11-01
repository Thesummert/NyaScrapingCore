# 调试器版本管理

使用 `cargo run -- <命令>` 或编译后的 `nsc <命令>`：

```text
list [openocd|pyocd|probe-rs]   列出已安装版本及 PATH 中的程序
install <工具> [版本]            安装指定版本；省略版本时安装最新版
uninstall <工具> <版本>          删除本工具管理的指定版本（也可用 remove）
use <工具> <版本|system>         保存选择；system 选择 PATH 中的第一个程序
which <工具>                     输出当前选择的可执行文件路径
paths                            显示配置文件及安装目录
```

例如：

```sh
nsc install openocd
nsc install pyocd 0.36.0
nsc install probe-rs 0.31.0
nsc use openocd 0.12.0-7
nsc use pyocd system
nsc list
nsc uninstall openocd 0.12.0-7
```

省略版本时会查询对应软件源的最新稳定版，OpenOCD 选择包含当前平台二进制文件的最新 xPack 正式发布。下载和构建进度由 curl、pip 或 Cargo 显示，具体形式取决于工具和终端；probe-rs 从源码构建，Cargo 的进度包含编译阶段。

`probers` 是 `probe-rs` 的别名。未设置选择时，`which` 使用 PATH 中的第一个匹配项；已选择的托管版本不存在时会报错。配置只影响本程序的解析，不会修改系统 PATH。`list` 检测 PATH 中所有同名程序并调用 `--version`；运行它们时请确保 PATH 中的程序可信。

卸载需要明确指定版本，只删除该版本的托管目录，其他版本及系统 PATH 中的程序不受影响。如果卸载的是当前选中版本，会清除该工具的选择，之后 `which` 将尝试使用 PATH 中的程序。

配置默认存于 Windows `%APPDATA%/NyaScrapingCore/config`、macOS `~/Library/Application Support/NyaScrapingCore/config`，或 Linux `$XDG_CONFIG_HOME/NyaScrapingCore/config`（默认 `~/.config`）。安装文件分别存于 Windows `%LOCALAPPDATA%/NyaScrapingCore/debuggers`、macOS `~/Library/Application Support/NyaScrapingCore/debuggers`，或 Linux `$XDG_DATA_HOME/NyaScrapingCore/debuggers`（默认 `~/.local/share`）。

OpenOCD 使用 [xPack 二进制发布包](https://xpack.github.io/openocd/install/)，版本需填写其发布版本，如 `0.12.0-7`；需要 `curl` 和支持该压缩格式的 `tar`。pyOCD 使用 [Python 虚拟环境和 pip](https://pyocd.io/docs/installing)，需要 Python 和 venv/pip。probe-rs 使用 [cargo install](https://probe.rs/docs/getting-started/installation/)，需要 Rust 工具链及平台构建依赖，编译可能耗时。安装均需要访问相应的软件源；版本和平台必须有对应的发行包。安装失败会清理本次创建的版本目录，不会覆盖已有目录。
