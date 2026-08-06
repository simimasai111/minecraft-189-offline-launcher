#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_bundle.py —— 在编译前生成本地内置资源 bundle.zip

输出位置：仓库根目录的 bundle.zip（与 Cargo.toml 同级），
          Rust 侧通过 include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/bundle.zip"))
          在编译期把整个离线游戏 + 便携 JRE 打进 exe。

bundle.zip 内部结构：
    game/versions/1.8.9/...   (客户端 jar + 版本 json)
    game/libraries/...        (33 个依赖 artifact)
    game/natives/...          (9 个 .dll)
    game/assets/...           (资源索引 1.8 + 722 个资源对象)
    jre/bin/java.exe ...      (Azul Zulu JRE 8，Windows x64)

用法：
    # 用本地已验证的离线目录加速（仅用于本机验证脚本正确性）
    GAME_SRC=/path/to/minecraft-offline python scripts/build_bundle.py
    # 在 GitHub Actions 中全自动从 Mojang + Azul 拉取（无需 GAME_SRC）
    python scripts/build_bundle.py
"""

import json
import os
import sys
import shutil
import zipfile
import hashlib
import subprocess
import tempfile

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # .../mclaunch
BUNDLE_DIR = os.path.join(REPO_ROOT, "bundle")
BUNDLE_ZIP = os.path.join(REPO_ROOT, "bundle.zip")
GAME_DIR = os.path.join(BUNDLE_DIR, "game")
JRE_DIR = os.path.join(BUNDLE_DIR, "jre")

VERSION = "1.8.9"
ASSET_ID = "1.8"
MANIFEST = "https://launchermeta.mojang.com/mc/game/version_manifest.json"
AZUL_API = (
    "https://api.azul.com/metadata/v1/zulu/packages/"
    "?java_version=8&java_package_type=jre&os=windows&arch=x64"
    "&archive_type=zip&release_status=ga&latest=true"
)


def curl(url, out, retries=5):
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    for _ in range(retries):
        try:
            r = subprocess.run(
                ["curl", "-sL", "--retry", "3", "--retry-all-errors", "-o", out, url],
                capture_output=True,
                text=True,
            )
            if r.returncode == 0 and os.path.exists(out) and os.path.getsize(out) > 0:
                return True
        except Exception as e:  # noqa: BLE001
            print("  curl exception:", e)
    return False


def sha1(path):
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for c in iter(lambda: f.read(1 << 16), b""):
            h.update(c)
    return h.hexdigest()


def copy_game(game_root, src):
    """从本地已验证的离线目录直接拷贝（加快本地验证）。"""
    os.makedirs(game_root, exist_ok=True)
    for name in ("versions", "libraries", "natives", "assets"):
        s = os.path.join(src, name)
        d = os.path.join(game_root, name)
        if os.path.isdir(s):
            if os.path.exists(d):
                shutil.rmtree(d)
            shutil.copytree(s, d)
    print("[ok] 已从本地拷贝游戏数据:", src)


def fetch_game(game_root):
    """全自动从 Mojang 官方拉取 1.8.9 离线资源。"""
    os.makedirs(game_root, exist_ok=True)
    lib = os.path.join(game_root, "libraries")
    nat = os.path.join(game_root, "natives")
    assets = os.path.join(game_root, "assets")
    obj = os.path.join(assets, "objects")
    idx = os.path.join(assets, "indexes")
    ver = os.path.join(game_root, "versions", VERSION)
    for d in (lib, nat, obj, idx, ver):
        os.makedirs(d, exist_ok=True)

    man_path = os.path.join(tempfile.gettempdir(), "mc_manifest.json")
    if not curl(MANIFEST, man_path):
        print("[FAIL] 无法下载 version_manifest.json"); sys.exit(1)
    man = json.load(open(man_path))
    vurl = None
    for e in man["versions"]:
        if e["id"] == VERSION:
            vurl = e["url"]
            break
    if not vurl:
        print("[FAIL] manifest 中找不到版本", VERSION); sys.exit(1)

    vjson = os.path.join(ver, VERSION + ".json")
    if not curl(vurl, vjson):
        print("[FAIL] 无法下载版本 json"); sys.exit(1)
    v = json.load(open(vjson))

    cj = v["downloads"]["client"]["url"]
    cjdest = os.path.join(ver, VERSION + ".jar")
    if not curl(cj, cjdest):
        print("[FAIL] 无法下载客户端 jar"); sys.exit(1)
    print("[ok] client jar 已下载")

    artifact_jars = []
    native_jars = []
    for lib_entry in v["libraries"]:
        dl = lib_entry.get("downloads", {})
        art = dl.get("artifact")
        if art:
            url = art["url"]
            path = art.get("path") or url.split("/")[-1]
            dest = os.path.join(lib, path)
            if not (os.path.exists(dest) and sha1(dest) == art.get("sha1", "")):
                if not curl(url, dest):
                    print("[FAIL] lib", path); sys.exit(1)
            if art.get("sha1") and sha1(dest) != art["sha1"]:
                print("[SHA1 MISMATCH] lib", path); sys.exit(1)
            artifact_jars.append(dest)
        natives = lib_entry.get("natives", {})
        win = natives.get("windows")
        if win and "classifiers" in dl:
            c = dl["classifiers"].get(win)
            if c:
                url = c["url"]
                path = c.get("path") or url.split("/")[-1]
                dest = os.path.join(lib, path)
                if not (os.path.exists(dest) and os.path.getsize(dest) > 0):
                    if not curl(url, dest):
                        print("[FAIL] native", path); sys.exit(1)
                native_jars.append(dest)
                exclude = lib_entry.get("extract", {}).get("exclude", [])
                with zipfile.ZipFile(dest) as z:
                    for n in z.namelist():
                        if n.lower().endswith(".dll") and not any(
                            n.startswith(e) for e in exclude
                        ):
                            z.extract(n, nat)
    print("[ok] 依赖: %d 个 artifact, %d 个 native jar" % (len(artifact_jars), len(native_jars)))

    idx_url = v["assetIndex"]["url"]
    idx_path = os.path.join(idx, ASSET_ID + ".json")
    if not curl(idx_url, idx_path):
        print("[FAIL] 无法下载资源索引"); sys.exit(1)
    idxj = json.load(open(idx_path))
    objs = idxj.get("objects", {})
    base = "https://resources.download.minecraft.net"
    done = fail = 0
    for _rel, meta in objs.items():
        h = meta["hash"]
        dest = os.path.join(obj, h[:2], h)
        if os.path.exists(dest) and os.path.getsize(dest) == meta.get("size", -1):
            done += 1
            continue
        if not curl("%s/%s/%s" % (base, h[:2], h), dest):
            fail += 1
            continue
        done += 1
    print("[ok] 资源对象: %d 成功, %d 失败" % (done, fail))


def fetch_jre(jre_dir):
    os.makedirs(jre_dir, exist_ok=True)
    apij = os.path.join(tempfile.gettempdir(), "azul.json")
    if not curl(AZUL_API, apij):
        print("[FAIL] 无法访问 Azul API"); sys.exit(1)
    pkgs = json.load(open(apij))
    url = None
    for p in pkgs:
        n = p.get("name", "")
        if (
            n.startswith("zulu8")
            and "-ca-jre8." in n
            and "-fx-" not in n
            and n.endswith("win_x64.zip")
        ):
            url = p["download_url"]
            break
    if not url:
        for p in pkgs:
            if (
                p.get("java_package_type") == "jre"
                and "win_x64" in p.get("name", "")
                and "-fx-" not in p.get("name", "")
            ):
                url = p["download_url"]
                break
    if not url:
        print("[FAIL] 未找到合适的 JRE 8 包"); sys.exit(1)
    print("[jre] 下载:", url)
    zpath = os.path.join(tempfile.gettempdir(), "zulu_jre.zip")
    if not curl(url, zpath):
        print("[FAIL] 无法下载 JRE"); sys.exit(1)
    with zipfile.ZipFile(zpath) as z:
        names = z.namelist()
        top = names[0].split("/")[0] if names else ""
        for n in names:
            if n.endswith("/"):
                continue
            rel = n[len(top) + 1:] if n.startswith(top + "/") else n
            if not rel:
                continue
            out = os.path.join(jre_dir, rel)
            os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
            with z.open(n) as src, open(out, "wb") as dst:
                shutil.copyfileobj(src, dst)
    java_exe = os.path.join(jre_dir, "bin", "java.exe")
    if not os.path.exists(java_exe):
        print("[FAIL] 解压后找不到", java_exe); sys.exit(1)
    print("[ok] JRE 8 已解压至", jre_dir)


def make_zip():
    print("[zip] 正在打包 bundle.zip ...")
    if os.path.exists(BUNDLE_ZIP):
        os.remove(BUNDLE_ZIP)
    with zipfile.ZipFile(BUNDLE_ZIP, "w", zipfile.ZIP_DEFLATED) as z:
        for dp, _dirs, files in os.walk(BUNDLE_DIR):
            for fn in files:
                fp = os.path.join(dp, fn)
                arc = os.path.relpath(fp, BUNDLE_DIR).replace("\\", "/")
                z.write(fp, arc)
    sz = os.path.getsize(BUNDLE_ZIP)
    print("[ok] bundle.zip = %.1f MB" % (sz / 1024 / 1024))


def main():
    src = os.environ.get("GAME_SRC", "")
    if os.path.isdir(src):
        copy_game(GAME_DIR, src)
    else:
        fetch_game(GAME_DIR)
    fetch_jre(JRE_DIR)
    make_zip()
    print("DONE")


if __name__ == "__main__":
    main()
