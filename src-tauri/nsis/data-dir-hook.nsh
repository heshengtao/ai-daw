; 安装时选择「数据目录」。
;
; 由 `tauri.conf.json` 的 `bundle.windows.nsis.installerHooks` 引入，挂在正式复制文件之前。
; 选择结果写进 HKCU\Software\AI DAW\DataDir，应用启动时由 `paths.rs` 读取（优先级低于
; 设置页写的指针文件）。这里**不**在静默安装 / 升级时弹窗：`/S` 或已有安装的自动升级
; 不该被一个人机交互卡住。
;
; 用户取消时什么都不写，应用回落到系统默认的 %APPDATA%\com.aidaw.app。

!macro NSIS_HOOK_PREINSTALL
  ${IfNot} ${Silent}
    ReadRegStr $R9 HKCU "Software\AI DAW" "DataDir"
    ; 只在**第一次安装**时问：升级时已有设置，别拿弹窗打断自动更新。
    ${If} $R9 == ""
      nsDialogs::SelectFolderDialog "选择 AI DAW 数据目录（模型 / 运行时 / 插件 / AI 产物）。建议新建一个专用空文件夹并放到空间充足的磁盘；取消则使用系统默认位置。" "$LOCALAPPDATA"
      Pop $R9

      ${If} $R9 != "error"
        ${If} $R9 != ""
          CreateDirectory "$R9"
          WriteRegStr HKCU "Software\AI DAW" "DataDir" "$R9"
        ${EndIf}
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend

; 卸载时保留用户数据：模型和工程缓存可能还有用，删不删交给用户。
!macro NSIS_HOOK_PREUNINSTALL
!macroend
