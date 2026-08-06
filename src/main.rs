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

fn http_get_text(url: &str) -> Result<String, String> {
    let tmp = std::env::temp_dir().join("mclaunch_api_tmp.json");
    download(url, tmp.to_str().unwrap())?;
    std::fs::read_to_string(&tmp).map_err(|e| e.to_string())
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

// ===================== Java 解析（支持捆绑 JRE） =====================
fn is_java8(java: &str) -> bool {
    if let Ok(out) = Command::new(java).args(["-version"]).output() {
        let t = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return t.contains("1.8");
    }
    false
}

fn find_bundled_java(game_dir: &str) -> Option<String> {
    let jre = Path::new(game_dir).join("jre");
    if let Ok(es) = std::fs::read_dir(&jre) {
        for e in es.flatten() {
            let p = e.path();
            if p.is_dir() {
                let c = p.join("bin").join("java.exe");
                if c.exists() {
                    return Some(c.to_str().unwrap().to_string());
                }
                if let Ok(es2) = std::fs::read_dir(&p) {
                    for e2 in es2.flatten() {
                        let p2 = e2.path();
                        if p2.is_dir() {
                            let c2 = p2.join("bin").join("java.exe");
                            if c2.exists() {
                                return Some(c2.to_str().unwrap().to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn resolve_java(cfg_java: &str, game_dir: &str) -> Result<String, String> {
    let mut cands: Vec<String> = Vec::new();
    if !cfg_java.is_empty() && cfg_java != "java" {
        cands.push(cfg_java.to_string());
    }
    if let Some(b) = find_bundled_java(game_dir) {
        cands.push(b);
    }
    cands.push("java".to_string());
    for c in &cands {
        if is_java8(c) {
            if c != "java" {
                println!("[java] 使用: {}", c);
            }
            return Ok(c.clone());
        }
    }
    if !cands.is_empty() {
        return Ok(cands[0].clone());
    }
    Err("未找到任何 Java 运行环境。请安装 Java 8，或在 config.toml 的 java 字段指定路径，或运行 `mclaunch setup` 下载捆绑 JRE。".to_string())
}

// ===================== setup: 下载离线资源 + 捆绑 JRE =====================
fn download_jre(game_dir: &str) -> Result<(), String> {
    let jre_dir = Path::new(game_dir).join("jre");
    std::fs::create_dir_all(&jre_dir).map_err(|e| e.to_string())?;
    if find_bundled_java(game_dir).is_some() {
        println!("[setup] 捆绑 JRE 已存在，跳过。");
        return Ok(());
    }
    println!("[setup] 查询便携 JRE 8 (Azul Zulu) ...");
    let api = "https://api.azul.com/metadata/v1/zulu/packages/?java_version=8&os=windows&arch=x64&archive_type=zip&java_package_type=jre&release_status=ga&latest=true";
    let s = http_get_text(api)?;
    let arr: serde_json::Value =
        serde_json::from_str(&s).map_err(|e| format!("解析 Azul API 失败: {}", e))?;
    let url = arr
        .get(0)
        .and_then(|v| v.get("download_url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| "无法从 Azul API 获取 JRE 下载地址".to_string())?;
    let zip_path = jre_dir.join("jre.zip");
    println!("[setup] 下载 JRE 8 (约 40-60MB) ...");
    download(url, zip_path.to_str().unwrap())?;
    println!("[setup] 解压 JRE ...");
    let _ = Command::new("tar")
        .args(["-xf", zip_path.to_str().unwrap(), "-C", jre_dir.to_str().unwrap()])
        .status();
    let _ = std::fs::remove_file(&zip_path);
    if find_bundled_java(game_dir).is_some() {
        println!("[setup] 捆绑 JRE 就绪（位于 jre/，无需另装 Java）。");
        Ok(())
    } else {
        Err("JRE 解压后未找到 java.exe，请检查 jre/ 目录。".to_string())
    }
}

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
        tasks.push((
            format!("{}/{}/{}", base, &h[0..2], h),
            dest.to_str().unwrap().to_string(),
        ));
    }
    println!("[setup] 并行下载 {} 个资源对象...", tasks.len());
    download_all(tasks)?;

    // 捆绑 JRE（自包含关键一步）
    download_jre(game_dir)?;

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
    cfg_java: &str,
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

    // 解析 Java（优先捆绑 JRE，其次 config，再次系统 java）
    let java = resolve_java(cfg_java, game_dir)?;
    if !is_java8(&java) {
        return Err(
            "找到的 Java 不是 8（1.8.x 必须 Java 8）。请在 config.toml 的 java 字段指向 Java 8，或运行 `mclaunch setup` 下载捆绑 JRE。".to_string(),
        );
    }

    // classpath = 客户端 jar + 所有依赖库 jar
    let mut cp_parts: Vec<String> = vec![client_jar.to_str().unwrap().to_string()];
    collect_jars(&lib_dir, &mut cp_parts);
    let cp = cp_parts.join(";");

    let mut cmd = Command::new(&java);
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
    let status = cmd.status().map_err(|e| e.to_string())?;

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
    println!(" 用法: mclaunch setup  (首次下载离线资源 + 捆绑 JRE)");
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
