//! openh264 真编码后端构建脚本（feature `openh264` 开启时执行）。
//!
//! 把 `third_party/openh264`（裁剪的 Cisco openh264 2.4.1 codec 源码 + LanDesk C ABI
//! 桥接层）编译成静态库，供 Rust FFI 链接。**不使用 `cc` crate**（vendor 无该依赖），
//! 直接 `std::process::Command` 调平台 C++ 编译器，保持 `--offline --locked` 门纯净。
//!
//! 关键：openh264 各模块有重名头文件（如 encoder/decoder 各有 deblocking.h），必须
//! 按上游 Makefile 的 per-module include 顺序编译，不能简单收集所有 inc 目录。
//!
//! - macOS / Linux / Windows-GNU：clang++ / g++ 编译 .cpp → ar 归档；
//! - Windows MSVC：cl.exe /c 编译 → lib.exe 归档。
//!
//! 纯 C++ 编译（USE_ASM=No），无需 nasm。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Common,
    EncoderCore,
    EncoderPlus,
    DecoderCore,
    DecoderPlus,
    Processing,
    Bridge,
}

fn main() {
    if env::var_os("CARGO_FEATURE_OPENH264").is_none() {
        println!("cargo:rerun-if-changed=build.rs");
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let oh_root = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("third_party")
        .join("openh264");
    let codec = oh_root.join("codec");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let obj_dir = out.join("oh264_obj");
    fs::create_dir_all(&obj_dir).expect("create obj dir");

    let mut sources: Vec<(PathBuf, Kind)> = Vec::new();
    collect_cpp(&codec, &mut sources);
    sources.push((oh_root.join("bridge.cpp"), Kind::Bridge));

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    if target_env == "msvc" {
        build_msvc(&sources, &obj_dir, &out, &codec);
    } else {
        let cxx = env::var("CXX").unwrap_or_else(|_| {
            if target_os == "macos" {
                "clang++".into()
            } else {
                "g++".into()
            }
        });
        build_gnu(&cxx, &sources, &obj_dir, &out, &codec, &target_os, &target_arch);
    }

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=oh264");
    if target_env == "msvc" {
        // MSVC 的 C++ 运行时由 cl.exe /MD 自动链接（msvcprt），显式 stdc++ 会链接失败。
    } else if target_os == "linux" {
        println!("cargo:rustc-link-lib=stdc++");
    } else {
        println!("cargo:rustc-link-lib=c++"); // macOS / FreeBSD：libc++
    }
    println!("cargo:rerun-if-changed={}", oh_root.display());
}

/// 复刻上游 Makefile 的 per-module include 顺序（避免重名头文件错配）。
fn includes_for(kind: Kind, codec: &Path, gnu: bool) -> Vec<String> {
    let flag = if gnu { "-I" } else { "/I" };
    let mut incs = vec![
        codec.join("api").join("wels"),
        codec.join("common").join("inc"),
    ];
    match kind {
        // 上游 ENCODER_INCLUDES：core/inc + plus/inc + processing/interface。
        // wels_preprocess.h 会 #include "IWelsVP.h"（在 processing/interface），
        // 因此 encoder/core 也必须带 processing/interface，否则 IWelsVP.h 缺失。
        Kind::EncoderCore | Kind::EncoderPlus => {
            incs.push(codec.join("encoder").join("core").join("inc"));
            incs.push(codec.join("encoder").join("plus").join("inc"));
            incs.push(codec.join("processing").join("interface"));
        }
        // 上游 DECODER_INCLUDES：core/inc + plus/inc。
        // 不与 encoder 混用，避免重名 deblocking.h 错配。
        Kind::DecoderCore | Kind::DecoderPlus => {
            incs.push(codec.join("decoder").join("core").join("inc"));
            incs.push(codec.join("decoder").join("plus").join("inc"));
        }
        // 上游 PROCESSING_INCLUDES：interface + 各 src 子目录。
        Kind::Processing => {
            let p = codec.join("processing");
            incs.push(p.join("interface"));
            incs.push(p.join("src").join("common"));
            incs.push(p.join("src").join("adaptivequantization"));
            incs.push(p.join("src").join("downsample"));
            incs.push(p.join("src").join("scrolldetection"));
            incs.push(p.join("src").join("vaacalc"));
        }
        // Common/Bridge 只需公共 INCLUDES（api/wels + common/inc）。
        Kind::Common | Kind::Bridge => {}
    }
    incs.into_iter()
        .map(|p| format!("{flag}{}", p.display()))
        .collect()
}

fn collect_cpp(dir: &Path, out: &mut Vec<(PathBuf, Kind)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name == "build" || name == "console" {
                    continue; // JNI 测试 / 示例程序，排除
                }
                collect_cpp(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("cpp") {
                let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if fname == "DllEntry.cpp" {
                    continue; // Windows DLL 入口，静态库不需要（上游仅 BUILD_DLL 编译）
                }
                // 按路径组件判断模块，不能字符串匹配 "/encoder/"——Windows 上
                // Path::display() 用反斜杠，会全部误判为 Common 导致 include 错配。
                let comps: Vec<String> = p
                    .components()
                    .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
                    .collect();
                let has = |seg: &[&str]| {
                    comps.windows(seg.len()).any(|w| {
                        w.iter().zip(seg.iter()).all(|(a, b)| a == b)
                    })
                };
                let kind = if has(&["encoder", "plus"]) {
                    Kind::EncoderPlus
                } else if has(&["encoder"]) {
                    Kind::EncoderCore
                } else if has(&["decoder", "plus"]) {
                    Kind::DecoderPlus
                } else if has(&["decoder"]) {
                    Kind::DecoderCore
                } else if has(&["processing"]) {
                    Kind::Processing
                } else {
                    Kind::Common
                };
                out.push((p, kind));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
}

fn common_defines(gnu: bool) -> Vec<&'static str> {
    // 不定义 GENERATED_VERSION_HEADER：version.h 会走默认版本串分支，
    // 不再依赖上游 Makefile 生成的 version_gen.h（该文件被 .gitignore 忽略、未入库，
    // CI 无此文件会编译失败）。版本串仅用于日志，不影响编码功能。
    if gnu {
        vec!["-O2", "-std=c++17", "-fPIC", "-DNDEBUG"]
    } else {
        vec!["/nologo", "/O2", "/std:c++17", "/DNDEBUG", "/EHsc"]
    }
}

fn build_gnu(
    cxx: &str,
    sources: &[(PathBuf, Kind)],
    obj_dir: &Path,
    out: &Path,
    codec: &Path,
    target_os: &str,
    target_arch: &str,
) {
    // macOS（Apple clang）用 -arch 指定目标架构。arm64 runner 交叉编 x86_64 时，
    // clang++ 默认产出宿主 arm64 .o，链接器忽略后符号全丢（v0.4.0 缺 Intel dmg 根因）。
    // 非 macOS 平台忽略：GNU 交叉靠 CC/CXX 前缀工具链（如 aarch64-linux-gnu-g++）。
    let arch_flag: Vec<&str> = if target_os == "macos" {
        match target_arch {
            "x86_64" => vec!["-arch", "x86_64"],
            "aarch64" => vec!["-arch", "arm64"],
            _ => vec![],
        }
    } else {
        vec![]
    };

    let mut objects = Vec::new();
    for (i, (src, kind)) in sources.iter().enumerate() {
        let obj = obj_dir.join(format!("oh264_{i}.o"));
        let mut cmd = Command::new(cxx);
        for f in common_defines(true) {
            cmd.arg(f);
        }
        for f in &arch_flag {
            cmd.arg(f);
        }
        cmd.arg("-c").arg(src);
        for inc in includes_for(*kind, codec, true) {
            cmd.arg(inc);
        }
        cmd.arg("-o").arg(&obj);
        let status = cmd.status().expect("spawn C++ compiler");
        assert!(status.success(), "C++ compile failed: {}", src.display());
        objects.push(obj);
    }
    let ar = env::var("AR").unwrap_or_else(|_| "ar".into());
    let lib = out.join("liboh264.a");
    let _ = fs::remove_file(&lib);
    let mut cmd = Command::new(ar);
    cmd.arg("cr").arg(&lib);
    for o in &objects {
        cmd.arg(o);
    }
    let status = cmd.status().expect("spawn ar");
    assert!(status.success(), "ar archive failed");
}

fn build_msvc(sources: &[(PathBuf, Kind)], obj_dir: &Path, out: &Path, codec: &Path) {
    let cl = env::var("CXX").unwrap_or_else(|_| "cl.exe".into());
    let mut objects = Vec::new();
    for (i, (src, kind)) in sources.iter().enumerate() {
        let obj = obj_dir.join(format!("oh264_{i}.obj"));
        let mut cmd = Command::new(&cl);
        for f in common_defines(false) {
            cmd.arg(f);
        }
        cmd.arg("/c").arg(src);
        for inc in includes_for(*kind, codec, false) {
            cmd.arg(inc);
        }
        cmd.arg(format!("/Fo{}", obj.display()));
        let status = cmd.status().expect("spawn cl.exe");
        assert!(status.success(), "MSVC compile failed: {}", src.display());
        objects.push(obj);
    }
    let lib = out.join("oh264.lib");
    let _ = fs::remove_file(&lib);
    let mut cmd = Command::new("lib.exe");
    cmd.arg("/nologo").arg(format!("/OUT:{}", lib.display()));
    for o in &objects {
        cmd.arg(o);
    }
    let status = cmd.status().expect("spawn lib.exe");
    assert!(status.success(), "lib.exe archive failed");
}
