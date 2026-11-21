#!/bin/sh
# deb 卸载/升级后刷新缓存，清掉启动器里失效的 LanDesk 入口与图标。
set -e

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

exit 0
