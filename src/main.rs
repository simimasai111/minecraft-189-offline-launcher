use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

// ===================== 配置 =====================
#[derive(Debug, Deserialize)]
struct Config {
    game_dir: Option<String>,
    java: Option<String>,
    username: Option<String>,
    max_ram_mb: Option<u32>,
    version: Option<String>,
    asset_index: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            game_dir: Some(".".into()),
            java: Some("java".into()),
            username: Some("Player".into()),
            max_ram_mb: Some(2048),
            version: Some("1.8.9".into()),
            asset_index: Some("1.8".into()),
        }
    }
}

fn load_config() -> Config {
    let path = "config.toml";
    match std::fs::read_to_string(path) {
        Ok(s) => match toml::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[警告] 解析 config.toml 失败，使用默认配置: {}", e);
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

// ===================== 版本清单结构 =====================
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionFile {
    asset_index: AssetIndexRef,
    libraries: Vec<Library>,
    main_class: String,
}

#[derive(Debug, Deserialize)]
struct AssetIndexRef {
    id: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct Library {
    downloads: Option<Downloads>,
    natives: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct Downloads {
    artifact: Option<Artifact>,
    classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Debug, Deserialize)]
struct Artifact {
    url: String,
    path: Option<String>,
}

// ===================== 资源索引结构 =====================
#[derive(Debug, Deserialize)]
struct AssetIndexFile {
    objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
}

// ===================== 下载工具 =====================
fn download(url: &str, dest: &str) -> Result<(), String> {
    if let Some(parent) = Path::new(dest).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
    }
    let status = Command::new("curl")
        .args(["-sL", "--retry", "3", "--retry-all-errors", "-o", dest, url])
        .status()
        .map_err(|e| format!("spawn curl: {}", e))?;
    if !status.success() {
        return Err(format!("curl 失败 ({}) -> {}", status, url));
    }
    Ok(())
}

fn download_all(tasks: Vec<(String, String)>) -> Result<(), String> {
    let mut handles = Vec::new();
    for (url, dest) in tasks {
        handles.push(std::thread::spawn(move || download(&url, &dest)));
    }
    let mut errs: Vec<String> = Vec::new();
    for h in handles {
        if let Err(e) = h.join().unwrap_or(Ok(())) {
            errs.push(e);
        }
    }
    if !errs.is_empty() {
        return Err(errs.join("\n"));
    }
    Ok(())
}

// 1.8.9 官方稳定地址（来自 launchermeta）
const VERSION_JSON_URL: &str =
    "https://piston-meta.mojang.com/v1/packages/d546f1707a3f2b7d034eece5ea2e311eda875787/1.8.9.json";
const CLIENT_JAR_URL: &str =
    "https://launcher.mojang.com/v1/objects/3870888a6c3d349d3771a3e9d16c9bf5e076b908/client.jar";

// ===================== setup: 下载离线资源 =====================
fn setup(game_dir: &str, version: &str, asset_index: &str) -> Result<(), String> {
    let gd = Path::new(game_dir);
    let ver_dir = gd.join("versions").join(version);
    let lib_dir = gd.join("libraries");
    let nat_dir = gd.join("natives");
    let idx_dir = gd.join("assets").join("indexes");
    let obj_dir = gd.join("assets").join("objects");
    for d in [&ver_dir, &lib_dir, &nat_dir, &idx_dir, &obj_dir] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }

    let ver_json_path = ver_dir.join(format!("{}.json", version));
    let client_jar_path = ver_dir.join(format!("{}.jar", version));

    println!("[setup] 下载版本清单与客户端...");
    download(VERSION_JSON_URL, ver_json_path.to_str().unwrap())?;
    download(CLIENT_JAR_URL, client_jar_path.to_str().unwrap())?;

    let json_str = std::fs::read_to_string(&ver_json_path).map_err(|e| e.to_string())?;
    let vf: VersionFile = serde_json::from_str(&json_str).map_err(|e| e.to_string())?;

    // 依赖库
    let mut tasks: Vec<(String, String)> = Vec::new();
    for lib in &vf.libraries {
        if let Some(dl) = &lib.downloads {
            if let Some(art) = &dl.artifact {
                let p = art
                    .path
                    .clone()
                    .unwrap_or_else(|| art.url.rsplit('/').next().unwrap().to_string());
                let dest = lib_dir.join(&p);
                tasks.push((art.url.clone(), dest.to_str().unwrap().to_string()));
            }
        }
    }
    println!("[setup] 并行下载 {} 个依赖库...", tasks.len());
    download_all(tasks)?;

    // 原生包 -> 解压 dll
    let mut native_pkgs: Vec<PathBuf> = Vec::new();
    for lib in &vf.libraries {
        if let Some(natives) = &lib.natives {
            if let Some(win_key) = natives.get("windows") {
                if let Some(dl) = &lib.downloads {
                    if let Some(cls) = &dl.classifiers {
                        if let Some(art) = cls.get(win_key) {
                            let p = art
                                .path
                                .clone()
                                .unwrap_or_else(|| art.url.rsplit('/').next().unwrap().to_string());
                            let dest = lib_dir.join(&p);
                            download(&art.url, dest.to_str().unwrap())?;
                            native_pkgs.push(dest);
                        }
                    }
                }
            }
        }
    }
    println!("[setup] 解压 {} 个原生包到 natives/...", native_pkgs.len());
    for pkg in &native_pkgs {
        let _ = Command::new("tar")
            .args(["-xf", pkg.to_str().unwrap(), "-C", nat_dir.to_str().unwrap()])
            .status();
    }

    // 资源索引 + 资源对象
    let idx_path = idx_dir.join(format!("{}.json", asset_index));
    println!("[setup] 下载资源索引 {}...", asset_index);
    download(&vf.asset_index.url, idx_path.to_str().unwrap())?;
    let idx_str = std::fs::read_to_string(&idx_path).map_err(|e| e.to_string())?;
    let idx: AssetIndexFile = serde_json::from_str(&idx_str).map_err(|e| e.to_string())?;
    let base = "https://resources.download.minecraft.net";
    let mut tasks: Vec<(String, String)> = Vec::new();
    for (_rel, obj) in &idx.objects {
        let h = obj.hash.as_str();
        let dest = obj_dir.join(&h[0..2]).join(h);
        tasks.push((format!("{}/{}/{}", base, &h[0..2], h), dest.to_str().unwrap().to_string()));
    }
    println!("[setup] 并行下载 {} 个资源对象...", tasks.len());
    download_all(tasks)?;

    println!("[setup] 完成！离线游戏目录已就绪：{}", gd.display());
    Ok(())
}

// ===================== launch: 启动游戏 =====================
fn collect_jars(dir: &Path, out: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_jars(&p, out);
            } else if let Some(ext) = p.extension() {
                if ext == "jar" {
                    out.push(p.to_str().unwrap().to_string());
                }
            }
        }
    }
}

fn launch(
    game_dir: &str,
    java: &str,
    username: &str,
    max_ram: u32,
    version: &str,
    asset_index: &str,
) -> Result<(), String> {
    let gd = Path::new(game_dir);
    let ver_dir = gd.join("versions").join(version);
    let client_jar = ver_dir.join(format!("{}.jar", version));
    let lib_dir = gd.join("libraries");
    let nat_dir = gd.join("natives");

    if !client_jar.exists() {
        return Err(format!(
            "找不到客户端 {}。请先运行 `mclaunch setup` 下载离线资源。",
            client_jar.display()
        ));
    }

    // Java 8 检测
    let out = Command::new(java)
        .args(["-version"])
        .output()
        .map_err(|e| format!("无法运行 java（{}）：{}", java, e))?;
    let ver_txt = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !ver_txt.contains("1.8") {
        return Err(
            "需要 Java 8 才能运行 Minecraft 1.8.9。\n请安装 Java 8，并在 config.toml 的 java 字段指向其 java.exe。"
                .to_string(),
        );
    }

    // classpath = 客户端 jar + 所有依赖库 jar
    let mut cp_parts: Vec<String> = vec![client_jar.to_str().unwrap().to_string()];
    collect_jars(&lib_dir, &mut cp_parts);
    let cp = cp_parts.join(";");

    let mut cmd = Command::new(java);
    cmd.arg(format!("-Xmx{}M", max_ram))
        .arg("-XX:+UseConcMarkSweepGC")
        .arg("-XX:+CMSIncrementalMode")
        .arg("-XX:-UseAdaptiveSizePolicy")
        .arg("-Xmn128M")
        .arg(format!("-Djava.library.path={}", nat_dir.to_str().unwrap()))
        .arg("-cp")
        .arg(&cp)
        .arg("net.minecraft.client.main.Main")
        .arg("--username")
        .arg(username)
        .arg("--version")
        .arg(version)
        .arg("--gameDir")
        .arg(gd.to_str().unwrap())
        .arg("--assetsDir")
        .arg(gd.join("assets").to_str().unwrap())
        .arg("--assetIndex")
        .arg(asset_index)
        .arg("--uuid")
        .arg("00000000-0000-0000-0000-000000000000")
        .arg("--accessToken")
        .arg("null")
        .arg("--userProperties")
        .arg("{}")
        .arg("--userType")
        .arg("mojang");
    let status = cmd
        .status()
        .map_err(|e| e.to_string())?;

    if !status.success() {
        return Err(format!("游戏进程退出异常：{}", status));
    }
    Ok(())
}

// ===================== 入口 =====================
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("launch");

    // 启动时先打印横幅，确保双击也能立刻看到控制台
    println!("==============================================");
    println!(" Minecraft 1.8.9 离线启动器 (Rust)");
    println!(" 用法: mclaunch setup  (首次下载离线资源)");
    println!("       mclaunch        (直接启动游戏)");
    println!("==============================================");

    let cfg = load_config();
    let game_dir = cfg.game_dir.clone().unwrap_or_else(|| ".".into());
    let java = cfg.java.clone().unwrap_or_else(|| "java".into());
    let username = cfg.username.clone().unwrap_or_else(|| "Player".into());
    let max_ram = cfg.max_ram_mb.unwrap_or(2048);
    let version = cfg.version.clone().unwrap_or_else(|| "1.8.9".into());
    let asset_index = cfg.asset_index.clone().unwrap_or_else(|| "1.8".into());

    let result: Result<(), String> = match mode {
        "setup" => setup(&game_dir, &version, &asset_index),
        "launch" | _ => launch(&game_dir, &java, &username, max_ram, &version, &asset_index),
    };

    match &result {
        Ok(()) => println!("\n[完成] 操作成功。"),
        Err(e) => eprintln!("\n[错误] {}", e),
    }

    // 等待用户按键再退出，避免双击运行时窗口一闪而过、看不到输出
    println!("按回车键退出...");
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);

    std::process::exit(match &result {
        Ok(()) => 0,
        Err(_) => 1,
    });
}
