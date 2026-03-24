# 🌌 OmniTouch (全能触控)

![Platform](https://img.shields.io/badge/Platform-Windows_10%20%7C%2011-blue.svg)
![Language](https://img.shields.io/badge/Language-Rust-orange.svg)
![License](https://img.shields.io/badge/License-MIT-green.svg)

**OmniTouch** 是一款专为 Windows 触控设备（平板电脑、二合一轻薄本）打造的全局映射引擎。它能够将屏幕触控操作以零延迟的级别转换为键盘按键、鼠标位移，甚至是原生的底层 Xbox 360 手柄信号。

无论你是想在没有外接键盘的设备上游玩游戏，还是需要一个高度自定义的屏幕快捷键面板，OmniTouch 都能为你提供最极致的操控体验。

---

## ✨ 核心特性

- 🚀 **极高能效与零延迟**：采用纯 **Rust** 编写，直接调用底层 Win32 API 与 Direct2D/GDI+ 渲染机制。告别臃肿的 Electron 和 C# 框架，CPU/内存占用极低，丝毫不抢占游戏性能。
- 🎮 **驱动级手柄模拟**：内嵌集成了 `ViGEmBus` 驱动。OmniTouch 可以在系统中创建出一个虚拟的 Xbox 360 手柄，完美支持各大 3A 游戏。
- 🔮 **高级触发变体 (Variants)**：不仅支持普通的点击，还支持复杂的交互：
  - **单击保持 (Toggle)**：点击锁定状态。
  - **滑动触发 (Swipe)**：手指划过即可触发按键。
  - **鼠标摇杆 (Joystick)**：以摇杆控制鼠标位移。
  - **绝对触控板 (Touchpad)**：屏幕区域与真实鼠标指针绝对映射。
- 💎 **Windows 11 现代原生 UI**：深度集成系统特性，支持 Mica 毛玻璃材质、圆角窗口、深色模式以及拥有完美按压反馈的原生组件。

---

## 🛠️ 编译与构建说明

本项目依赖 C++ 编写的 `ViGEmClient` SDK，因此在编译前，请确保你的开发环境已安装相应的 C++ 编译工具链。

### 环境准备
1. 安装 [Rust 工具链 (rustup)](https://rustup.rs/)。
2. 安装 **Visual Studio C++ Build Tools**（在 Visual Studio Installer 中勾选“使用 C++ 的桌面开发”）。

### 一键编译
克隆本仓库到本地后，在根目录执行以下命令：
```bash
cargo build --release
```

## 目录结构说明

src/

├── main.rs (入口)

├── app\_state.rs     (全局状态)

├── config.rs     (本地存取)

├── input/     (所有的硬件模拟在这里)

│      ├── mod.rs

│      ├── handler.rs

│      └── vigem\_wrapper.rs

├── core/     (所有的业务逻辑与消息分发在这里)

│      ├── mod.rs

│      ├── event\_handler.rs

│      └── wndprocs.rs

└── ui/     (所有的视觉展现与画板在这里)

              ├── mod.rs

              ├── base.rs

              ├── render.rs

              └── panels.rs

