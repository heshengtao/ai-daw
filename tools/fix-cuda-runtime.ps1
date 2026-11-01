# 把 %APPDATA%\com.aidaw.app\runtime\ort\cuda 里**混在一起的两套文件**换成一致的
# ORT 1.26（CUDA 12）三件套，并补上 cuFFT 组件目录。
#
# 为什么需要它：排查过程中我先把 provider 换成了 1.26，但核心 dll 当时被运行中的应用占用、
# 没能替换，于是目录里成了「1.30 内核 + 1.26 provider」。这两者版本必须一致，
# 否则 provider 的 DllMain 直接失败（错误 1114），ORT 静默回落 CPU。
#
# 用法：**先退出 AI DAW**，然后在 PowerShell 里运行本脚本，再启动应用。
$ErrorActionPreference = "Stop"

if (Get-Process ai-daw -ErrorAction SilentlyContinue) {
    throw "请先退出 AI DAW（进程还在，dll 被占用无法替换）"
}

$src = "D:\ai-daw\.tmp\ort126"
$dst = Join-Path $env:APPDATA "com.aidaw.app\runtime\ort\cuda"
$cufftDir = Join-Path $env:APPDATA "com.aidaw.app\runtime\ort\cufft"

foreach ($f in @("onnxruntime.dll", "onnxruntime_providers_shared.dll", "onnxruntime_providers_cuda.dll")) {
    if (-not (Test-Path (Join-Path $src $f))) { throw "缺少 $src\$f" }
}
if (-not (Test-Path $dst)) { New-Item -ItemType Directory -Force $dst | Out-Null }

# 清掉我加的备份文件，避免下次又误用旧的 1.30
Get-ChildItem $dst -Filter "*.bak" -ErrorAction SilentlyContinue | Remove-Item -Force

Copy-Item (Join-Path $src "onnxruntime.dll") $dst -Force
Copy-Item (Join-Path $src "onnxruntime_providers_shared.dll") $dst -Force
Copy-Item (Join-Path $src "onnxruntime_providers_cuda.dll") $dst -Force

# cuFFT 是 ORT 1.26 CUDA provider 的硬依赖，装到它自己的组件目录
New-Item -ItemType Directory -Force $cufftDir | Out-Null
Copy-Item (Join-Path $src "cufft64_11.dll") $cufftDir -Force

Get-ChildItem $dst, $cufftDir | Select-Object FullName, Length | Format-Table -AutoSize

"完成：现在启动 AI DAW，它会按 ORT 1.26 + CUDA 12 组件加载，日志里应出现"
"[runtime] 已注册 CUDA EP 库（…\onnxruntime_providers_cuda.dll）"
