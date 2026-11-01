# NyaScrapingCore

NyaScrapingCore 是一个使用 Rust 编写的 MCU 调试器前端核心，通过命令行对接 OpenOCD、pyOCD 和 probe-rs。

## 当前状态

目前已实现调试器版本管理，以及三种工具的统一 GDB 服务、烧录、擦除、复位和 GDB 调试会话命令。命令与参数见 [DEBUGGERS.md](DEBUGGERS.md)。

## 项目结构

```text
NyaScrapingCore/
├── Cargo.toml          # Rust 包配置
├── readme.md
└── src/
    ├── main.rs         # 命令行程序入口
    ├── lib.rs          # 核心库入口
    ├── cli/            # 命令行解析
    ├── session/        # GDB 会话管理
    └── backend/        # 外部工具命令适配
        ├── openocd.rs
        ├── pyocd.rs
        └── probers.rs
```

核心库和程序入口分开，以便其他界面复用核心能力。各后端生成对应工具命令；会话模块管理 GDB 服务和客户端进程；命令行模块负责用户输入。

## 构建与运行

需要安装 Rust 工具链（包含 Cargo），然后在项目根目录执行：

```sh
cargo build
cargo run
```

不带参数运行会显示帮助。实际执行调试命令需要安装相应外部工具；可用 `nsc install` 管理版本，或使用 PATH 中的程序。

## 后续方向

- 按需扩展探针选择、速度设置及目标特有的调试选项。
- 添加真实探针和目标硬件上的集成测试。
