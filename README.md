# Minecraft 1.8.9 单文件离线启动器 (Rust)

一个 **单文件 `mclaunch.exe`** 即可离线玩《我的世界》Java 版 **1.8.9**：整个离线游戏资源（客户端、依赖库、原生 DLL、资源包）和便携 **JRE 8** 都已在编译期打进 exe 内部，双击即玩，**无需在系统里安装 Java，也无需联网下载任何东西**。

## 这个项目做了什么

- 用 Rust 编写启动器外壳，编译为单个 Windows exe。
- 编译期通过 `include_bytes!` 把 `bundle.zip`（= 完整离线游戏目录 + Azul Zulu JRE 8）直接嵌入 exe 二进制。
- 首次运行时，exe 自动把内置资源解压到同目录下的 `mclaunch-data/`（只需一次）；之后直接启动，**实现"秒启动"**。
- `bundle.zip` 由 `scripts/build_bundle.py` 在 GitHub Actions 构建阶段自动生成：
  - 游戏数据从 Mojang 官方源拉取（客户端 jar、33 个依赖库、9 个原生 DLL、资源索引 1.8 + 722 个资源对象）；
  - 便携 JRE 8 从 Azul Zulu 官方源拉取（Windows x64）。
- 通过 GitHub Actions 在 Windows 上自动构建并发布 exe。

> 说明：游戏逻辑本身仍是官方 Java 版（打进包里的 `client.jar`），Rust 负责"自解压 + 装配 classpath + 调起 JRE 启动"这一外壳。"秒启动"来自：零下载、内置调优过的 JRE、以及仅首次解压一次。把整套 Java 游戏逐行用 Rust 重写是不现实的，但我们做到了让最终交付物是"单个 exe、啥都不用装"。

## 用法

1. 下载 Release 里的 `mclaunch.exe`。
2. 双击运行。
   - 首次会自动解压内置资源到 `mclaunch-data/`（约十几秒，进度会在控制台显示）。
   - 之后每次双击都是秒开。
3. （可选）把 `config.toml.example` 复制为同目录 `config.toml`，改用户名 / 内存等。

控制台会显示解压进度、选用的 Java、以及任何错误信息；游戏退出后按回车关闭窗口。

## 配置文件（config.toml，可选）

| 字段         | 说明                                          | 默认     |
|--------------|-----------------------------------------------|----------|
| java         | Java 可执行文件路径（需 Java 8；留空用内置 JRE）| `java`   |
| username     | 离线用户名                                    | `Player` |
| max_ram_mb   | 最大内存 (MB)                                 | `2048`   |

## 构建（GitHub Actions 自动完成）

仓库根目录的 `bundle.zip` **不入库**，由工作流在编译前生成：

```bash
python scripts/build_bundle.py   # 生成 bundle.zip（游戏 + JRE）
cargo build --release            # exe 会嵌入 bundle.zip
```

本地手动构建同样需要这一步；`bundle.zip` 大约 160 MB，因此生成的 `mclaunch.exe` 也是约 160 MB 的自包含文件。

## 许可

MIT
