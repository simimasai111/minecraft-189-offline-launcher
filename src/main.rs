use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

// 编译时把整个离线游戏 + 便携 JRE 打进 exe（由 scripts/build_bundle.py 生成 bundle.zip）
const BUNDLE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/bundle.zip"));

// ===================== 配置 =====================
#[derive(Debug, Deserialize)]
struct Config {
    java: Option<String>,
    username: Option<String>,
    max_ram_mb: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            java: Some("java".into()),
            username: Some("Player".into()),
            max_ram_mb: Some(2048),
        }
    }
}

fn load_config() -> Config {
    match std::fs::read_to_string("config.toml") {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("[警告] 解析 config.toml 失败，使用默认配置: {}", e);
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

// ===================== Java 解析（优先内置 JRE） =====================
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

fn resolve_java(cfg_java: &str, bundled_jre: &Path) -> Result<String, String> {
    let mut cands: Vec<String> = Vec::new();
    if !cfg_java.is_empty() && cfg_java != "java" {
        cands.push(cfg_java.to_string());
    }
    if bundled_jre.exists() {
        cands.push(bundled_jre.to_str().unwrap().to_string());
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
    Err("未找到 Java 8 运行环境。".to_string())
}

// ===================== 首次解包内置资源 =====================
fn extract_bundle(data_dir: &Path) {
    fs::create_dir_all(data_dir).ok();
    println!("[首次启动] 正在解压内置资源（只需一次，请稍候）...");
    let cursor = io::Cursor::new(BUNDLE);
    let mut archive = zip::ZipArchive::new(cursor).expect("内置 bundle 损坏，请重新下载 exe");
    let total = archive.len();
    let mut n = 0u32;
    for i in 0..total {
        let mut f = archive.by_index(i).expect("读取内置 bundle 失败");
        let name = f.name().to_string();
        // 跨平台安全拼接：把 zip 内的 / 或 \ 拆成路径分量，避免 Windows 把 "/" 当字面字符
        let out = {
            let mut p = data_dir.to_path_buf();
            for part in name.split(|c| c == '/' || c == '\\') {
                if !part.is_empty() && part != "." {
                    p = p.join(part);
                }
            }
            p
        };
        if f.is_dir() {
            fs::create_dir_all(&out).ok();
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).ok();
            }
            let mut buf = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut buf).ok();
            fs::write(&out, &buf).ok();
        }
        n += 1;
        if n % 500 == 0 {
            println!("  解压进度 {}/{}", n, total);
        }
    }
    fs::write(data_dir.join(".extracted"), "1").ok();
    println!("[首次启动] 解压完成（{} 个文件）。", total);
}

// ===================== 启动游戏 =====================
fn collect_jars(dir: &Path, out: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
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
) -> Result<(), String> {
    let gd = Path::new(game_dir);
    let ver_dir = gd.join("versions").join("1.8.9");
    let client_jar = ver_dir.join("1.8.9.jar");
    let lib_dir = gd.join("libraries");
    let nat_dir = gd.join("natives");

    if !client_jar.exists() {
        return Err(format!("找不到客户端 {}。", client_jar.display()));
    }

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
        .arg("1.8.9")
        .arg("--gameDir")
        .arg(gd.to_str().unwrap())
        .arg("--assetsDir")
        .arg(gd.join("assets").to_str().unwrap())
        .arg("--assetIndex")
        .arg("1.8")
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
    println!("==============================================");
    println!(" Minecraft 1.8.9 单文件离线启动器 (Rust)");
    println!(" 双击即可游玩，无需安装 Java / 下载资源");
    println!("==============================================");

    // exe 所在目录，作为数据缓存根
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|x| x.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let data_dir = exe_dir.join("mclaunch-data");

    // 首次自动解包内置资源（游戏 + JRE）
    if !data_dir.join(".extracted").exists() {
        extract_bundle(&data_dir);
    }

    let game_dir = data_dir.join("game");
    let bundled_jre = data_dir.join("jre").join("bin").join("java.exe");

    let cfg = load_config();
    let java_cfg = cfg.java.clone().unwrap_or_else(|| "java".into());
    let username = cfg.username.clone().unwrap_or_else(|| "Player".into());
    let max_ram = cfg.max_ram_mb.unwrap_or(2048);

    let result: Result<(), String> = (|| {
        let java = resolve_java(&java_cfg, &bundled_jre)?;
        if !is_java8(&java) {
            return Err(
                "内置 JRE 异常（非 Java 8）。请重新下载本启动器，或删除 mclaunch-data 目录后重试。".to_string(),
            );
        }
        launch(
            game_dir.to_str().unwrap(),
            &java,
            &username,
            max_ram,
        )
    })();

    match &result {
        Ok(()) => println!("\n[完成] 游戏已退出。"),
        Err(e) => eprintln!("\n[错误] {}", e),
    }

    println!("按回车键退出...");
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
    std::process::exit(match &result {
        Ok(()) => 0,
        Err(_) => 1,
    });
}
