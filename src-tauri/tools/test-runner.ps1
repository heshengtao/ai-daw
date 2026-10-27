# Cargo 的 target runner：给 Windows 的测试可执行文件补一份 comctl32 v6 清单，然后照常运行。
#
# 背景：`tauri-plugin-dialog` → `rfd` 静态导入 `comctl32!TaskDialogIndirect`；测试可执行
# 文件没有应用清单时，Windows 绑到 System32 的 comctl32 5.82（没有该导出），进程在跑第一
# 个用例前就死了（STATUS_ENTRYPOINT_NOT_FOUND / 0xc0000139）。
#
# 不这么做的话只能在链接期嵌清单，而那会连带影响正式 App（见 .cargo/config.toml 的说明）。
# 这里只处理 `target\<profile>\deps\` 下的测试壳；`cargo run` 之类原样穿透。
#
# 注意：本文件必须以 **UTF-8 BOM** 保存——Windows PowerShell 5.1 对无 BOM 的脚本按 ANSI
# 解码，中文注释会变成乱码甚至语法错误。

param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Argv)

$ErrorActionPreference = 'Stop'

$exe = $Argv[0]
$rest = @()
if ($Argv.Count -gt 1) { $rest = $Argv[1..($Argv.Count - 1)] }

# 只有测试壳需要补：它们躺在 target\...\deps\ 下。
if ($exe -match '\\deps\\') {
    $mtPath = $null
    $mt = Get-Command mt.exe -ErrorAction SilentlyContinue
    if ($mt) {
        # ApplicationInfo 没有 FullName，命令路径在 Source（即 Path）上。
        $mtPath = $mt.Source
    }
    if (-not $mtPath) {
        # 没有在 PATH 上就按 Windows SDK 的常规位置找最新的 x64 版。
        $found = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\mt.exe' -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($found) { $mtPath = $found.FullName }
    }

    $manifest = Join-Path $PSScriptRoot '..\win-test.manifest'
    if ($mtPath -and (Test-Path $manifest)) {
        # mt 原地改写 PE 的资源段；重复嵌入同一份清单是幂等的。
        & $mtPath -nologo -manifest $manifest -outputresource:"$exe;#1" | Out-Null
    }
}

& $exe @rest
exit $LASTEXITCODE
