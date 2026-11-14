#!/bin/sh
# 容器入口：先起一块"没有屏幕的屏幕"（Xvfb），再交给 daemon。
#
# 为什么需要它：插件（VST3 / CLAP）的原生界面是 X11 窗口。容器里没有窗口系统，
# 但插件照样能在一个虚拟显示上画界面；daemon 把这块屏读出来编码成帧流推给浏览器，
# 页面里就是"插件窗口在网页内"。没有 Xvfb 的话，带界面的插件一开窗就失败。
#
# 关掉：`AIDAW_GUI_STREAM=0`（省内存；此时插件只能改参数，界面上看不到）。
set -e

if [ "${AIDAW_GUI_STREAM:-1}" = "1" ] && command -v Xvfb >/dev/null 2>&1; then
    : "${DISPLAY:=:99}"
    export DISPLAY
    # 尺寸要与 daemon 的 `AIDAW_GUI_SIZE` 一致（默认 1440x900）。
    : "${AIDAW_GUI_SIZE:=1440x900}"
    W=${AIDAW_GUI_SIZE%x*}
    H=${AIDAW_GUI_SIZE#*x}
    Xvfb "$DISPLAY" -screen 0 "${W}x${H}x24" -nolisten tcp >/tmp/xvfb.log 2>&1 &
    # 等 X 上电：xdotool 连得上就绪（失败就继续，插件界面会是错误而不是卡死）。
    i=0
    while [ "$i" -lt 50 ]; do
        if DISPLAY="$DISPLAY" xdotool getdisplaygeometry >/dev/null 2>&1; then
            break
        fi
        i=$((i + 1))
        sleep 0.1
    done
    export AIDAW_GUI_SIZE
fi

exec aidawd --data-dir /data --lan --port 28430
