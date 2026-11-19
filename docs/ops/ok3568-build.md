# OK3568 交叉编译（aarch64）

> 对应 `docs/PRD.md` NFR-04 / 风险表#4（libav 裁剪交叉编译麻烦）。
> OK3568 为 Allwinner aarch64（Linux），目标 `aarch64-unknown-linux-gnu`，静态链接单文件（NFR-05）。

## 1. 工具链

```bash
# 1) Rust target
rustup target add aarch64-unknown-linux-gnu

# 2) aarch64 交叉 gcc/链接器（Debian/Ubuntu 主机）
sudo apt install gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu
```

`.cargo/config.toml` 已含交叉链接器配置：

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
# 静态链接（OK3568 单文件部署）
rustflags = ["-C", "target-feature=+crt-static"]
```

## 2. 裁剪 libav（x86_64 vs aarch64 差异）

| 项 | x86_64 | aarch64 / OK3568 |
|---|---|---|
| 硬编通路 | qsv + nvenc + amf + v4l2m2m | 仅 **v4l2m2m**（Allwinner RISC-V/VPU）|
| 软编兜底 | OpenH264 | OpenH264（交叉编译）|
| 产物 | `thirdparty/av-x86_64` | `thirdparty/av-aarch64` |

### 编译脚本（`scripts/build-libav/`）
```bash
# x86_64
./scripts/build-libav/build-x86_64.sh

# OK3568 aarch64（含 v4l2m2m + 软编）
CC=aarch64-linux-gnu-gcc ./scripts/build-libav/build-aarch64-ok3568.sh

# OpenH264 软编（两平台各编一份）
CC=aarch64-linux-gnu-gcc ./scripts/build-libav/build-openh264.sh
```

脚本约定输出到 `thirdparty/<arch>/`（`.gitignore` 已忽略 `thirdparty/`），
`LAVF_ROOT` 环境变量指向该目录供 `avcodec-wrap/build.rs` 探测。

## 3. 构建

```bash
LAVF_ROOT=thirdparty/av-aarch64 \
OPENH264_ROOT=thirdparty/av-aarch64 \
cargo build --release --target aarch64-unknown-linux-gnu \
  -p controlled -p controller \
  --features "capture/linux,avcodec-wrap/v4l2m2m,avcodec-wrap/openh264"
```

## 4. 部署到 OK3568 开发板

```bash
# 静态链接单文件（NFR-05，无外部 .so 依赖）
scp target/aarch64-unknown-linux-gnu/release/controlled root@<board>:~/landesk/
ssh root@<board> 'chmod +x ~/landesk/controlled && ~/landesk/controlled --role controlled'
```

### 验证要点（对齐 PRD §9 验收 #6）
1. `file` 确认产物为 aarch64 静态二进制；
2. `ldd` 应显示「statically linked」；
3. 起服务后 `lsof -i` 看 UDP 52000 监听；
4. 编码器日志确认走 `v4l2m2m`，失败自动降级 `openh264`。

## 5. 交叉编译容器（可选）

`scripts/cross-ok3568/Dockerfile` 提供 `cross` 容器环境，避免主机手装 aarch64 sysroot。
