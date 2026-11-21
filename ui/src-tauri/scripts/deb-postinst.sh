#!/bin/sh
# LanDesk deb/rpm 安装或升级后处理：
#   1) 刷新系统 desktop 数据库与 hicolor 图标缓存；
#   2) 为每个本机用户补一份用户级 application 条目
#      —— 这是启动器（GNOME Shell / KDE / XFCE）直接扫描的来源，
#         不依赖 mimeinfo 缓存是否刷新成功，最可靠；
#   3) 为每个本机用户在桌面创建 LanDesk 快捷方式
#      —— deb/rpm 安装默认不会创建桌面图标，必须显式做。
# 所有步骤失败都不阻断安装（不影响主程序可用性）。
set -u

DESKTOP_SRC="/usr/share/applications/LanDesk.desktop"
DESKTOP_NAME="LanDesk.desktop"

[ -f "$DESKTOP_SRC" ] || exit 0

# ---- 1) 系统级缓存刷新（命令缺失则跳过，不影响后续步骤）----
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi
for _gtk_cache in \
  gtk-update-icon-cache \
  gtk-update-icon-cache-3.0 \
  gtk-update-icon-cache-4.0
do
  if command -v "$_gtk_cache" >/dev/null 2>&1; then
    "$_gtk_cache" -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
    break
  fi
done

# ---- 收集需要处理的本机用户 ----
# sudo/软件中心安装时以 $SUDO_USER 为准；再遍历 /home 覆盖多用户与直接 root 安装场景。
_users=""
if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
  _users="$SUDO_USER"
fi
for _h in /home/*; do
  [ -d "$_h" ] || continue
  _u=$(basename "$_h")
  case " $_users " in
    *" $_u "*) continue ;;
  esac
  # 只处理有有效 passwd 记录的用户（排除不被系统识别的残留目录）
  if getent passwd "$_u" >/dev/null 2>&1; then
    _users="$_users $_u"
  fi
done

# ---- 每个用户：用户级 application 条目 + 桌面快捷方式 ----
for _u in $_users; do
  _home=$(getent passwd "$_u" 2>/dev/null | cut -d: -f6)
  [ -n "$_home" ] && [ -d "$_home" ] || continue

  # 2) 用户级 application 条目：XDG 规范，启动器直接扫描，无需刷新系统缓存
  _app_dir="$_home/.local/share/applications"
  if mkdir -p "$_app_dir" 2>/dev/null; then
    cp -f "$DESKTOP_SRC" "$_app_dir/$DESKTOP_NAME" 2>/dev/null || true
    chown -R "$_u" "$_app_dir" 2>/dev/null || true
  fi

  # 3) 桌面快捷方式：优先解析 XDG_DESKTOP_DIR（支持中文「桌面」等本地化目录名）
  _desk=""
  if [ -r "$_home/.config/user-dirs.dirs" ]; then
    _raw=$(sed -n 's/^XDG_DESKTOP_DIR="\([^"]*\)".*/\1/p' "$_home/.config/user-dirs.dirs" | head -1)
    case "$_raw" in
      '$HOME'*)     _desk="$_home/${_raw#\$HOME}" ;;
      '${HOME}'*)   _desk="$_home/${_raw#\${HOME}}" ;;
      /*)           _desk="$_raw" ;;
      *)            _desk="" ;;
    esac
    # 前缀展开后可能残留双斜杠（$HOME 本身以 / 结尾或变量后紧跟 /），规整化
    case "$_desk" in *"//"*) _desk=$(printf '%s' "$_desk" | sed 's#//*#/#g') ;; esac
  fi
  [ -n "$_desk" ] || _desk="$_home/Desktop"
  if mkdir -p "$_desk" 2>/dev/null; then
    cp -f "$DESKTOP_SRC" "$_desk/$DESKTOP_NAME" 2>/dev/null || true
    chmod +x "$_desk/$DESKTOP_NAME" 2>/dev/null || true
    chown "$_u" "$_desk/$DESKTOP_NAME" 2>/dev/null || true
    # GNOME 会把未标记的桌面 launcher 当作「不受信任」，标记后双击可直接运行。
    if command -v gio >/dev/null 2>&1; then
      su - "$_u" -c "gio set '$_desk/$DESKTOP_NAME' metadata::trusted true" >/dev/null 2>&1 || true
    fi
  fi
done

exit 0
