# wayscrollshot

一个用于 Wayland 的滚动截图工具，在滚动时实时捕获并拼接图像。
![preview](./preview.png)

[English](README.md)

## 功能特性

- 实时预览与自动拼接
- 列采样算法实现快速精准的重叠检测
- 圆角按钮 UI 与悬停效果（基于 tiny-skia）
- 键盘快捷键与鼠标控制
- 保存到文件或复制到剪贴板
- 支持反向滚动(仅列采样算法)

## 工作原理

### 架构

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   捕获模块   │────>│   拼接模块   │────>│   预览模块   │
│   (grim)    │     │ (列采样算法) │     │ (layer-shell)│
└─────────────┘     └──────────────┘     └─────────────┘
```

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
| `slurp` | 区域选择 | 是 |
| `grim` | 屏幕捕获 | 是 |
| OpenCV | 默认拼接算法 | 是 |
| `wl-copy` | 剪贴板（Wayland） | 剪贴板功能需要 |
| `xclip` | 剪贴板（X11 回退） | 备选方案 |

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

### 从源码构建

```bash
# 安装运行时和构建依赖（Arch Linux）
sudo pacman -S slurp grim wl-clipboard opencv clang libxkbcommon

# 构建
cargo build --release

# 安装（可选）
cp target/release/wayscrollshot ~/.local/bin/
```

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

# 使用不同的拼接算法
wayscrollshot -a opencv-orb  # 默认：ORB 特征匹配 + RANSAC
wayscrollshot -a col-sample  # 快速列采样
wayscrollshot -a template    # 模板匹配（更精确）
wayscrollshot -a edge        # 边缘检测（适用于透明背景）
wayscrollshot -a fast        # FAST 角点 + HNSW 索引（实验性）
```

### 选项

| 选项 | 描述 | 默认值 |
|------|------|--------|
| `-o, --output <PATH>` | 输出文件路径 | `~/Pictures/wayscrollshot/wayscrollshot-<时间戳>.png` |
| `-w, --preview-width <PX>` | 预览宽度（像素） | 280 |
| `-c, --clipboard` | 复制到剪贴板而非保存 | false |
| `--no-preview` | 禁用预览窗口 | false |
| `--no-border` | 禁用区域边框覆盖层 | false |
| `-a, --algorithm <ALG>` | 拼接算法：`opencv-orb`、`col-sample`、`template`、`edge`、`fast` | opencv-orb |

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

## 局限性

1. **仅支持 Wayland**：不支持 X11。本工具使用 `wlr-layer-shell-unstable-v1` 协议。

2. **基于 wlroots 的合成器**：在 Sway、Hyprland、river 等上可用。可能无法在 GNOME/KDE Wayland 上运行。

3. **重叠要求**：每次滚动必须与前一视图保留一些重叠。滚动过快可能导致拼接失败。

4. **静态内容假设**：算法假设滚动内容是静态的。动态内容（动画、视频）会产生伪影。

5. **仅支持垂直滚动**：目前不支持水平滚动。

6. **固定页眉/页脚**：如果页面有固定的页眉或页脚，它们会被重复捕获。建议选择排除它们的区域。

## 故障排除

### "slurp selection failed"
- 确保 `slurp` 已安装且在 PATH 中
- 检查是否在 Wayland 上运行

### "layer-shell not available"
- 你的合成器不支持 `wlr-layer-shell-unstable-v1`
- 尝试使用基于 wlroots 的合成器（Sway、Hyprland）

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
