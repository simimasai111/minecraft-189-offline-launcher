# Minecraft 1.8.9 离线启动器 (Rust)

用 Rust 重写的 Minecraft Java 版 **1.8.9** 离线启动器：配置驱动、单文件 exe、由 GitHub Actions 自动编译发布。

相比原本的 `launch.bat` + 手工下载脚本，这个版本：

- 用 Rust 重写，**下载更轻更快**（多线程并行拉取依赖库与资源对象）。
- 一个 `config.toml` 控制所有参数（游戏目录、Java 路径、用户名、内存等）。
- 编译出**单个 `mclaunch.exe`**，双击即可启动，无需批处理。
- 通过 GitHub Actions 在 Windows 上自动构建并发布 exe。

## 用法

```text
mclaunch setup    # 首次：下载客户端、依赖库、原生文件、资源到 game_dir
mclaunch          # 启动游戏（默认）
```

1. 把 `config.toml.example` 复制为 `config.toml`，按需修改。
2. 运行 `mclaunch setup` 完成离线资源下载，**并自动下载一个捆绑的便携 JRE 8 到 `jre/`**（只需一次，之后断网也能玩）。
3. 双击 `mclaunch.exe`（或直接 `mclaunch`）启动——**无需在系统里单独安装 Java**，启动器会优先使用 `jre/` 里的捆绑 JRE。

> 如果你更想用系统已装的 Java 8，在 `config.toml` 的 `java` 字段写完整路径即可，启动器会优先采用。

## 配置文件（config.toml）

| 字段         | 说明                                   | 默认     |
|--------------|----------------------------------------|----------|
| game_dir     | 游戏目录（含 versions/libraries/...）  | `.`      |
| java         | Java 可执行文件路径（需 Java 8）       | `java`   |
| username     | 离线用户名                             | `Player` |
| max_ram_mb   | 最大内存 (MB)                          | `2048`   |
| version      | 游戏版本                               | `1.8.9`  |
| asset_index  | 资源索引                               | `1.8`    |

## 构建（本地）

```bash
cargo build --release
# 产物：target/release/mclaunch.exe
```

代码本身不打包 Minecraft 资源文件；首次运行 `setup` 会从官方源下载并落地到 `game_dir`。

## 许可

MIT
