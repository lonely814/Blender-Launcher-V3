# Blender Launcher V3

用 **Tauri 2 + Rust + SvelteKit** 重写的 Blender 版本管理器。

这不是 [Blender-Launcher-V2](https://github.com/Victor-IX/Blender-Launcher-V2) 的皮肤层移植，而是一次完全重写：后端从 Python/PySide6 换成 Rust，前端从 Qt Widgets 换成 Web 技术栈。原版的「三 Tab + 竖排子页签」结构被保留下来作为 *Classic* 布局，另有两套全新的布局可选。

> 本项目是重写版本，不是原版的官方分支。原版仍在 Victor-IX 维护。GPL-3.0 授权，见 [LICENSE](./LICENSE)。

---

## 它做什么

管理本机上的多个 Blender 版本：从 builder.blender.org 抓取每日/实验/稳定构建列表，下载解压到本地库，一键启动、切换、删除。

| 能力 | 说明 |
|---|---|
| 版本抓取 | Stable / Daily / Experimental / Patch，外加 Bforartists、UPBGE、UPBGE Weekly；只列本机平台的便携归档（Windows `.zip`、macOS `.dmg`、Linux `.tar.*`），安装器（`msi`/`msix`）不进列表 |
| 下载管理 | 并发下载、暂停/恢复/取消、进度与 ETA、断点续传、清华 TUNA 镜像、镜像内容按官方 sha256 校验 |
| 本地库扫描 | 识别 `.blinfo`，未知目录弹 Build Entry Creator 补全元数据 |
| 更新检测 | 比对已装版本与远端，行内出现更新图标，点击下载新版 |
| 启动与实例 | 启动/调试启动、运行中实例计数、`.blend` 文件关联与拖放打开 |
| 自定义构建 | 手填版本/哈希/分支，或跑 `blender -v` 自动探测 |
| 插件管理 | 第四个 Tab：读已装扩展与旧式插件、官方目录浏览/更新/安装、版本历史回滚（中央库 + 链接切换） |
| 集成 | 系统托盘、全局快捷键、文件关联、快捷方式、符号链接、WinGet ARP 登记 |
| 多语言 | 英语 / 简体中文，跟随系统或手动指定 |
| 主题 | 暗色 / 亮色 / 跟随系统，5 套强调色，Windows 11 Mica 材质 |

## 三套布局

| 布局 | 结构 | 适用 |
|---|---|---|
| **Classic** | 左侧 68px 竖排页签 + 表格 + 右侧传输栏 | 原版习惯 |
| **Wide** | 顶部横向页签 + 紧凑行（28px），表格吃满宽度 | 宽屏、版本多 |
| **Detail** | Wide + 右侧 280px 详情面板（哈希、路径、发布说明） | 需要核对构建信息 |

在 设置 → 外观 → 布局 中切换。主题、强调色、语言、Mica 也在同一页。

---

## 技术栈

**后端** Rust 2021 · Tauri 2 · tokio · reqwest · rusqlite · soup

**前端** Svelte 5（runes）· SvelteKit 2（`ssr = false`）· Tailwind 3 · TypeScript

几个值得一提的实现决定：

- **主题走 CSS 变量**：Tailwind 颜色全部写成 `rgb(var(--surface-2) / <alpha-value>)`，切 `[data-theme]` 就换整套配色，`/60` 这类透明度修饰符照常工作，组件类名一个都不用改。
- **列表虚拟滚动**：701 行只挂载约 30 个 DOM 节点，窗口计算抽成纯函数 `src/lib/virtual.ts`，有 12 条边界断言。行高是常量（40 / 28px），所以没引入 TanStack Virtual。
- **SSR 关闭**：`getCurrentWindow()` 读 `window.__TAURI_INTERNALS__`，在 Node 侧不存在。对一个没有 SEO 需求的本地 SPA，SSR 只会带来这类崩溃。
- **单实例**：`File::try_lock` 拿锁，进程退出由内核释放，第二个实例只请求焦点然后退出。

---

## 构建

**依赖** Node.js 20+ · Rust 1.89+（用到 `File::try_lock`）· Windows 10/11

```bash
npm install
npm run tauri:dev      # 开发，热重载
npm run tauri:build    # 产出单文件 exe（bundle.active = false）
```

产物在 `src-tauri/target/release/blender-launcher-v3.exe`，约 16 MB。
WebView2 用系统运行时，不打包进 exe；Win10 1803 之前的机器需要先装
WebView2 Runtime。

**其他命令**

```bash
npm run check                      # svelte-check（类型 + a11y）
node tests/virtual.test.mjs        # 虚拟滚动窗口计算断言
cargo test --manifest-path src-tauri/Cargo.toml
```

> 构建前先结束正在运行的实例。Windows 会锁住 exe，`tauri build` 会以
> `failed to remove file ... (os error 5)` 失败。

## 便携模式

exe 旁边放一个名为 `portable` 的空文件，配置、缓存、构建库就全部落在
exe 同级目录，整个文件夹可以拷走：

```
Blender Launcher/
├── blender-launcher-v3.exe
├── portable              ← 空标记文件
├── settings.json         ← 自动生成
├── cache/                ← SQLite 缓存、单实例锁
└── builds/               ← 构建库
    ├── stable/  daily/  experimental/
    ├── bforartists/  upbge-stable/  upbge-weekly/
    ├── custom/  template/
```

判定依据是**标记文件是否存在**，而不是「exe 所在目录可写」——后者会随
UAC 和 Program Files 权限漂移，同一个 exe 换台机器数据位置就变了。
没有标记文件时走系统目录：配置在 `%APPDATA%\BlenderLauncher\`，缓存在
`%LOCALAPPDATA%\BlenderLauncher\`，默认库在
`%LOCALAPPDATA%\Blender Launcher\`。

`builds/` 下 8 个子目录在首次启动时就建好（含 `template`，Blender 的启动
模板目录），与原版 `create_library_folders()` 一致。


---

## 项目结构

```
src/
  routes/
    +layout.svelte        主题 / 语言 / Mica 落地
    +layout.ts            ssr = false
    +page.svelte          主界面（三套布局在此分支）
  lib/
    components/           16 个组件
    i18n/                 类型化字典，en / zh
    virtual.ts            虚拟滚动窗口计算（纯函数）
    menu-position.ts      右键菜单视口钳制（纯函数）
    stores.ts             全局状态 + 后端事件接线
    theme.ts              主题解析与 DOM 落地
src-tauri/
  src/
    commands/             42 个 IPC 命令
    service/              爬虫、下载器、解压、库扫描、启动器
    models/               Build / Download / Settings
    db/                   SQLite 缓存
    single_instance.rs    文件锁单实例
tests/
  virtual.test.mjs        12 条断言
```

---

## 已知限制

- **仅 Windows 验证过**。代码里有 macOS / Linux 的路径分支，但没有实机测试。
- **WinGet 未发布清单**。目前只写应用和功能（ARP）条目，**不**写 `WinGetPackageIdentifier`——写上别家作者的包 ID 会让 `winget upgrade` 去拉别人仓库的内容覆盖本机安装。
- **代理密码明文存储**，在 `settings.json` 里。界面有提示，但没接系统密钥环。
- **Mica 仅 Windows 11 22H2+**，其他系统自动降级为不透明底。
- 中文字形不打包，回退到系统字体（微软雅黑等）。

## 致谢

原版 [Blender-Launcher-V2](https://github.com/Victor-IX/Blender-Launcher-V2)（Victor-IX）及其前身 [Blender-Launcher](https://github.com/DotBow/Blender-Launcher)（DotBow）。本项目的交互结构、`.blinfo` 格式与数据源解析大量参考原版。
