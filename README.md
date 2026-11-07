# QuotaDock

**Windows 原生 Codex 额度悬浮组件。** 在屏幕边缘查看 7 天 / 5 小时剩余额度，用灵动岛通知和状态丝带掌握任务进度。

Rust · Win32 · GDI+ · Windows 10/11 · MIT

![QuotaDock 默认主题](docs/images/theme-default-expanded.png)

## 功能

- **额度一目了然**：同时显示两组剩余额度、理想进度刻度、偏差与重置时间。
- **融合吸附**：上下边缘显示横条，左右边缘显示竖条，拖离边缘恢复紧凑 HUD。
- **平滑展开**：悬停查看详情，双击切换完整窗口；通知结束恢复原布局。
- **任务提醒**：默认监听本地会话完成记录，展示项目、任务名与回复摘要，Hook 为可选通道。
- **状态灯与丝带**：灰色无任务、蓝色运行中、绿色完成、橙色中断；独立开关，支持四档丝带强度。
- **五套主题**：经典默认、深海蓝、松林绿、暮光紫、暖砂棕。
- **可保存设置**：摘要字数和行数上限、置顶、缩放、底板样式、通知来源与刷新间隔。
- **原生窗口**：当前构建不需要 Electron、Tauri 或 WebView2 Runtime。

## 配色预览

| 深海蓝 | 松林绿 |
| --- | --- |
| ![深海蓝](docs/images/theme-ocean-expanded.png) | ![松林绿](docs/images/theme-forest-expanded.png) |

| 暮光紫 | 暖砂棕 |
| --- | --- |
| ![暮光紫](docs/images/theme-violet-expanded.png) | ![暖砂棕](docs/images/theme-amber-expanded.png) |

### 任务通知

![任务通知](docs/images/theme-default-notice.png)

以上为程序绘制的示例数据预览，不代表实时额度。

## 开始使用

1. 在 Windows 10/11 上安装、登录 Codex CLI，确保本机可以运行 `codex`。额度读取依赖本机 CLI；任务通知默认直接读取桌面会话记录。
2. 从源码构建 `dist/QuotaDock.exe`；有发布版本后，也可从仓库 Releases 下载 EXE。
3. 双击 EXE。左键托盘图标显示 / 隐藏窗口，右键打开设置。

常用操作：拖动移动、悬停展开、双击切换完整窗口、滚轮调整缩放。关闭窗口会隐藏到托盘，彻底退出请使用菜单中的“退出 QuotaDock”。

### 托盘菜单

| 菜单 | 内容 |
| --- | --- |
| 窗口布局 | 紧凑 HUD、顶部细条、融合模式、完整窗口、始终置顶 |
| 外观与托盘 | 颜色主题、紧凑 HUD 深色底板、托盘图标精度 |
| 任务状态与丝带 | 状态灯、条形 / 点状、边缘丝带、丝带强度 |
| 通知设置 | 任务结束通知、摘要与行数上限、额度重置提醒、预览 |
| Codex 集成 | 权限确认、任务接收方式、可选 Hook 设置 |
| 常规设置 | 开机启动、额度刷新间隔 |

## 从源码构建

需要 Windows、Rust GNU 工具链，以及 PATH 中的 MinGW `gcc` 和 `windres`。

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup override set stable-x86_64-pc-windows-gnu
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1
```

首次构建需要联网下载 Rust 依赖。依赖已缓存时，可在构建命令末尾加 `-Offline`。输出为 `dist/QuotaDock.exe`，不需要运行 `npm install`。

构建脚本会关闭当前工作目录 `dist/QuotaDock.exe` 的运行实例以替换文件；构建结束后需要重新启动。

完整环境配置见 [构建说明](docs/BUILD.md)。

## 数据与兼容性

- 设置、额度缓存和通知去重记录保存在 `%APPDATA%\cn.codex.quota.widget\`。
- 额度来自本机 Codex CLI 的 `account/rateLimits/read`；可用 `CODEX_CLI_PATH` 指定 CLI 路径。
- 本地任务记录默认读取 `~/.codex/sessions`，支持 `CODEX_HOME` 指定位置。QuotaDock 不主动上传会话和通知摘要。
- 任务通知不要求安装 Hook；可选 Hook 首次使用仍需由用户在 Codex 中审阅并信任。
- 本轮结束也可能是等待用户回复，不代表整个需求完成；未写出中断事件的崩溃、断网不能可靠识别。
- 云端任务没有本地记录时无法通过本地监听显示通知。Codex 更改协议或记录格式后可能需要适配。

更多功能及 Hook 操作见 [使用说明](docs/USAGE.md)。

## 项目结构

```text
src-native/       当前 Rust / Win32 实现与 Cargo.lock
src/             历史 JavaScript 界面和节奏分析实现
src-tauri/       历史 Tauri 实现（当前构建不使用）
scripts/         构建、源码整理、回归验证脚本
assets/          程序图标与历史界面资源
docs/            使用、构建、发布说明和界面预览
```

编译缓存、个人 Codex 配置、测试记录、诊断产物和 EXE 均不纳入源码仓库。EXE 适合作为 GitHub Release 附件发布。

## 参与与许可

提交问题请附上系统版本、缩放比例、窗口模式和复现步骤；不要附带登录凭据或完整会话。开发说明见 [CONTRIBUTING.md](CONTRIBUTING.md)。

本项目以 [MIT License](LICENSE) 分发，基于既有 Codex 额度组件演进，上游署名及依赖说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。QuotaDock 与 OpenAI 没有官方关联。
