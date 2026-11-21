#!/bin/sh
# LanDesk deb/rpm 卸载后清理：
#   1) 删除安装脚本为各用户创建的用户级 application 条目与桌面快捷方式；
#   2) 刷新系统 desktop 数据库与 hicolor 图标缓存。
# 只精确删除 LanDesk.desktop，不触碰用户其它文件。所有步骤失败都不阻断卸载。
set -u

DESKTOP_NAME="LanDesk.desktop"

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
  if getent passwd "$_u" >/dev/null 2>&1; then
    _users="$_users $_u"
  fi
done

for _u in $_users; do
  _home=$(getent passwd "$_u" 2>/dev/null | cut -d: -f6)
  [ -n "$_home" ] && [ -d "$_home" ] || continue

  rm -f "$_home/.local/share/applications/$DESKTOP_NAME" 2>/dev/null || true

  _desk=""
  if [ -r "$_home/.config/user-dirs.dirs" ]; then
    _raw=$(sed -n 's/^XDG_DESKTOP_DIR="\([^"]*\)".*/\1/p' "$_home/.config/user-dirs.dirs" | head -1)
    case "$_raw" in
      '$HOME'*)     _desk="$_home/${_raw#\$HOME}" ;;
      '${HOME}'*)   _desk="$_home/${_raw#\${HOME}}" ;;
      /*)           _desk="$_raw" ;;
      *)            _desk="" ;;
    esac
    case "$_desk" in *"//"*) _desk=$(printf '%s' "$_desk" | sed 's#//*#/#g') ;; esac
  fi
  [ -n "$_desk" ] || _desk="$_home/Desktop"
  rm -f "$_desk/$DESKTOP_NAME" 2>/dev/null || true
done

exit 0
