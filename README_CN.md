# wayscrollshot

一个用于 Wayland 的滚动截图工具，在滚动时实时捕获并拼接图像。
![preview](./preview.png)

[English](README.md)

## 功能特性

- 实时预览与自动拼接
- 八带签名拼接，忽略静止侧栏以及固定顶栏、底栏
- 圆角按钮 UI 与悬停效果（基于 tiny-skia）
- 键盘快捷键与鼠标控制
- 保存到文件或复制到剪贴板
- 支持反向滚动（`band` 与列采样）

## 工作原理

### 架构

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   捕获模块   │────>│   拼接模块   │────>│   预览模块   │
│   (grim)    │     │    (band)    │     │ (layer-shell)│
└─────────────┘     └──────────────┘     └─────────────┘
```

### 默认拼接（`band`）

默认算法是 mark-shot 使用的八带行签名：

1. 在画面上取八条窄竖带，每一行得到一个亮度签名。
2. 把新帧和上一帧的同一行作比较。几乎不动的带视为静止侧栏，不参与匹配。
3. 只在运动带上搜索滚动量，然后只追加新出现的行。
4. 顶部或底部始终相同的行视为固定顶栏或底栏，不会重复贴上。

`-a col-sample` 仍然使用下面的列采样。

### 列采样算法

wayscrollshot 没有采用逐像素比较整张图像的方式，而是使用了受 [screenshot-splicing](https://github.com/baotlake/screenshot-splicing) 启发的列采样方法：

1. **从每帧采样 3 组列**：
   - 左侧区域（20 到 width/4）
   - 中间区域（width/2 到 5\*width/8）
   - 右侧区域（6\*width/8 到 7\*width/8）

2. **转换为灰度**并对每组取平均值

3. **使用平均绝对差（MAD）搜索重叠**：
   - 从预测偏移量开始（基于上次滚动）
   - 向外扩展搜索：`[p, p+1, p-1, p+2, p-2, ...]`
   - 当 MAD < 阈值时提前终止

4. **将新内容追加**到拼接图像

**复杂度**：O(9 \* height) 而非 O(width \* height) —— 显著提升速度。

### 重叠检测

```
帧 1（前一帧）：              帧 2（当前帧）：
┌────────────────┐           ┌────────────────┐
│    内容 A      │           │    内容 B      │
│                │           │                │
│    内容 B      │ <──────── │    内容 B      │  （重叠）
│                │           │                │
│    内容 C      │           │    内容 C      │
└────────────────┘           │                │
                             │    内容 D      │  （新增）
                             └────────────────┘
```

算法找到帧 2 顶部与帧 1 内容匹配的位置，然后仅追加新增部分。

## 依赖

### 运行时依赖

| 工具 / 库 | 用途 | 必需 |
|-----------|------|------|
| `slurp` | 区域选择 | 未传入 `REGION` 时需要 |
| `grim` | wlroots 合成器上的屏幕捕获 | 仅 wlroots（KDE 不使用） |
| OpenCV | 默认拼接算法 | 是 |
| `wl-copy` | 剪贴板（Wayland） | 剪贴板功能需要 |
| `xclip` | 剪贴板（X11 回退） | 备选方案 |

在 KDE Plasma 上，截图通过内置的 `org.kde.KWin.ScreenShot2` D-Bus 接口完成，不使用 `grim`；详见[合成器兼容性](#合成器兼容性)。

### 构建依赖

| Crate | 用途 |
|-------|------|
| `smithay-client-toolkit` | Wayland 客户端库 |
| `wayland-client` | Wayland 协议绑定 |
| `tiny-skia` | 2D 图形（圆角按钮） |
| `image` | 图像处理与缩放 |
| `opencv` | ORB 特征匹配与 RANSAC 对齐 |
| `clap` | 命令行参数解析 |
| `anyhow` | 错误处理 |
| `chrono` | 文件名时间戳 |
| `log` / `env_logger` | 日志记录 |

从源码构建需要安装 OpenCV 开发文件和 Clang。

## 安装

### Debian / Ubuntu

从 GitHub Releases 下载与 Ubuntu 版本和 CPU 架构匹配的 `.deb` 包，然后用 `apt` 安装：

```bash
sudo apt install ./wayscrollshot_0.1.7-1ubuntu26.04_amd64.deb
```

Debian 包会按 Ubuntu 版本分别构建，OpenCV 运行时依赖会匹配对应发行版的 ABI。不要通过软链接不同版本的 OpenCV 库绕过缺失库错误。

`apt install ./wayscrollshot_….deb` 会拉取 `grim`、`slurp` 以及包内记录的 OpenCV 共享库。不要执行 `apt install opencv clang`：这是 Arch 的包名，Ubuntu 没有名为 `opencv` 的包。如果用 `dpkg -i` 安装后提示缺库，执行 `sudo apt install -f`。

GNOME 需要随包安装的 Shell 扩展（Mutter 没有 layer-shell，`slurp` 和 `grim` 无法在那里工作）。安装后启用扩展并重新登录一次：

```bash
gnome-extensions enable wayscrollshot-scroll-helper@snemc.org
```

### 从源码构建

```bash
# Arch Linux
sudo pacman -S slurp grim wl-clipboard opencv clang libxkbcommon

# Debian / Ubuntu / Pop!_OS（包名与 Arch 不同）
sudo apt install build-essential pkg-config clang libclang-dev libopencv-dev libxkbcommon-dev slurp grim wl-clipboard

# 构建
cargo build --release

# 安装（可选）
cp target/release/wayscrollshot ~/.local/bin/
```

Ubuntu 上的 OpenCV 开发包是 `libopencv-dev`，不是 `opencv`。`clang` 只在编译时需要，安装好的 `.deb` 不依赖它。

### 从Nix构建

#### 运行

```shell
nix run github:jswysnemc/wayscrollshot
```

#### home-manager 配置

```nix
# input
inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  
  wayscrollshot = {
    url = "github:jswysnemc/wayscrollshot";
    inputs.nixpkgs.follows = "nixpkgs";
  };
}
```

## 使用方法

```bash
# 基本用法
wayscrollshot

# 保存到指定文件
wayscrollshot -o ~/screenshot.png

# 复制到剪贴板而非保存
wayscrollshot -c

# 自定义预览宽度
wayscrollshot -w 320

# 禁用预览窗口
wayscrollshot --no-preview

# 禁用区域边框覆盖层
wayscrollshot --no-border

# 使用已有 slurp 选择结果
wayscrollshot "$(slurp)"

# 从标准输入读取 slurp 输出
slurp | wayscrollshot -

# 使用不同的拼接算法
wayscrollshot -a band        # 默认：八带签名，忽略静止侧栏和固定顶栏、底栏
wayscrollshot -a opencv-orb  # ORB 特征匹配 + RANSAC
wayscrollshot -a col-sample  # 快速列采样
wayscrollshot -a template    # 模板匹配（更精确）
wayscrollshot -a edge        # 边缘检测（适用于透明背景）
wayscrollshot -a fast        # FAST 角点 + HNSW 索引（实验性）

# 选择捕获后端（默认自动检测）
wayscrollshot --backend grim   # wlroots：Sway、Hyprland、niri、river
wayscrollshot --backend kwin   # KDE Plasma（KWin ScreenShot2）
wayscrollshot --backend gnome  # GNOME Shell 助手扩展
```

### 选项

| 选项 | 描述 | 默认值 |
|------|------|--------|
| `-o, --output <PATH>` | 输出文件路径 | `~/Pictures/wayscrollshot/wayscrollshot-<时间戳>.png` |
| `-w, --preview-width <PX>` | 预览宽度（像素） | 280 |
| `-c, --clipboard` | 复制到剪贴板而非保存 | false |
| `--no-preview` | 禁用预览窗口 | false |
| `--no-border` | 禁用区域边框覆盖层 | false |
| `-a, --algorithm <ALG>` | 拼接算法：`band`、`opencv-orb`、`col-sample`、`template`、`edge`、`fast` | band |
| `--backend <BACKEND>` | 捕获后端：`auto`、`grim`、`kwin`、`gnome` | auto |
| `REGION` | 已有 slurp/grim 几何区域，例如 `10,20 300x400`；使用 `-` 从标准输入读取 | 交互运行 `slurp` |

### 控制方式

**鼠标：**
- 点击控制栏中的按钮

**控制栏按钮：**
| 按钮 | 作用 |
|------|------|
| `S`（保存） | 保存图片并退出 |
| `C`（复制） | 复制到剪贴板并退出 |
| `P`（暂停/继续） | 暂停或继续捕获 |
| `X`（取消） | 取消捕获并退出 |

**键盘（当覆盖层获得焦点时）：**
| 按键 | 操作 |
|------|------|
| `S` | 保存并退出 |
| `C` | 复制到剪贴板并退出 |
| `Space` | 暂停/继续捕获 |
| `Q` / `Esc` | 取消并退出 |

> **注意：** 键盘快捷键依赖合成器对 `wlr-layer-shell` `OnDemand` 键盘焦点的实现。在 niri 下工作良好；在 Hyprland 等合成器下可能需要将鼠标悬停在控制栏上才能触发。所有合成器均可通过鼠标点击控制栏按钮进行操作。

## 合成器兼容性

预览悬浮层与区域边框使用 `wlr-layer-shell-unstable-v1`，区域选择使用 `slurp`，屏幕捕获在运行时按合成器选择后端（可用 `--backend` 覆盖）。

| 桌面环境 | 区域选择 | 预览 / 边框 | 捕获后端 | 状态 |
|----------|----------|-------------|----------|------|
| wlroots（Sway、Hyprland、niri、river） | `slurp` | layer-shell | `grim` | 开箱即用 |
| KDE Plasma（KWin） | `slurp` | layer-shell | KWin ScreenShot2（D-Bus） | 可用，需完成下方授权步骤 |
| GNOME（Mutter） | Shell 助手 | Shell 助手按钮 | Shell 助手 `ScreenshotArea` | 需启用 `wayscrollshot-scroll-helper@snemc.org` 并重新登录 |

### KDE 授权

KWin 仅在调用进程拥有声明 `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` 的 `.desktop` 文件时，才允许其调用 `org.kde.KWin.ScreenShot2`。deb、AUR 与 Nix 包会安装 `wayscrollshot.desktop` 以满足此要求。运行自行构建的二进制时，请将该文件安装到 `/usr/share/applications/`（或 `~/.local/share/applications/`），或在临时测试时绕过检查：

```bash
KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 wayscrollshot
```

## 局限性

1. **仅支持 Wayland**：不支持 X11。预览与区域边框使用 `wlr-layer-shell-unstable-v1`（wlroots 合成器与 KWin 支持，GNOME 不支持）。

2. **桌面环境支持**：详见[合成器兼容性](#合成器兼容性)。KDE Plasma 通过 KWin ScreenShot2 支持。GNOME 通过 Shell 助手扩展支持；预览浮层改为保存、复制、暂停、取消按钮。

3. **重叠要求**：每次滚动必须与前一视图保留一些重叠。滚动过快可能导致拼接失败。

4. **静态内容假设**：算法假设滚动内容是静态的。动态内容（动画、视频）会产生伪影。

5. **仅支持垂直滚动**：目前不支持水平滚动。

6. **固定区域**：默认 `band` 算法会去掉不随页面滚动的顶栏、底栏和侧栏。高亮移动幅度明显小于正文的侧栏会按静止区域处理。如果某条栏仍然重复，请把选区收紧。

## 故障排除

### "Unable to locate package opencv"（或 `clang`）
- 那条命令是 Arch 的包列表。只想运行时，用 `sudo apt install ./wayscrollshot_….deb` 安装发布包。只有从源码编译时才需要 Ubuntu 上的 `libopencv-dev`。
- `grim` 和 `slurp` 位于 Ubuntu 的 `universe` 组件。

### "slurp selection failed"
- 在 GNOME 上这是预期情况：启用 `wayscrollshot-scroll-helper@snemc.org` 并重新登录一次。`slurp` 依赖 `wlr-layer-shell`，Mutter 没有实现它。
- 在 wlroots 和 KDE 上，确保 `slurp` 已安装且在 PATH 中
- 检查是否在 Wayland 上运行

### "layer-shell not available"
- 你的合成器不支持 `wlr-layer-shell-unstable-v1`
- 尝试使用基于 wlroots 的合成器（Sway、Hyprland）或 KDE Plasma

### "KWin denied the screenshot (NoAuthorized)"（KDE）
- 将 `wayscrollshot.desktop`（随包安装）放入标准应用目录；它必须声明 `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`
- 临时测试时，可用 `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` 运行

### "No overlap match"
- 滚动慢一些
- 确保帧之间有可见的重叠
- 避免滚动穿过完全不同的内容

### 预览不更新
- 检查捕获区域是否正确
- 尝试使用 `RUST_LOG=debug` 运行以获取更多信息

## 许可证

MIT

## 贡献

欢迎贡献！以下是可以改进的方向：

- **算法优化**：`fast` 算法（FAST 角点 + HNSW）需要调优以提高准确性
- **跨平台支持**：由于 Wayland 依赖，目前仅支持 Linux
- **性能优化**：减少超长截图的内存占用
- **UI 改进**：捕获过程中提供更好的视觉反馈

请在提交 PR 之前先开 issue 讨论重大更改。

## 致谢

- [screenshot-splicing](https://github.com/baotlake/screenshot-splicing) - 列采样算法灵感来源
- [snow-shot](https://github.com/mg-chao/snow-shot) - FAST 角点 + HNSW 算法参考
- [smithay-client-toolkit](https://github.com/Smithay/client-toolkit) - Wayland 客户端库
- [tiny-skia](https://github.com/RazrFalcon/tiny-skia) - 2D 图形库
