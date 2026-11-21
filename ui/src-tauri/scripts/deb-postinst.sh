#!/bin/sh
# deb 安装/升级后刷新桌面入口与图标缓存。
# 不这样做的话，desktop 文件与图标虽然装到了 /usr/share，
# 但 GNOME/KDE/XFCE 等桌面环境的启动器不会立即识别，表现为「装了却找不到图标」。
set -e

# update-desktop-database：重建 /usr/share/applications 的 mimeinfo 缓存，
# 让启动器能枚举到 LanDesk.desktop。desktop-file-utils 提供，缺失时跳过。
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

# gtk-update-icon-cache：重建 hicolor 主题缓存，让启动器/文件管理器显示 LanDesk 图标。
# 不同发行版可能只有 gtk-update-icon-cache 或带版本后缀。逐个尝试，都不存在则跳过。
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

exit 0
