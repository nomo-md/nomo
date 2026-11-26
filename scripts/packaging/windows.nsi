# -*- coding: utf-8 -*-
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"
!include "nsDialogs.nsh"
!include "WordFunc.nsh"
!include "StrFunc.nsh"
${StrTok}
!define PRODUCTNAME "Nomo"
!define MAINBINARYNAME "nomo"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Nomo"
!define SETUP_QUIT_MESSAGE 0x804f ; WM_APP + 0x4f in ui/tray/windows.rs
Var NomoInstalledVersion
Var NomoPreviousDir
Name "Nomo"
VIProductVersion "${NUMERICVERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "Nomo"
VIAddVersionKey /LANG=1033 "FileDescription" "Nomo installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "AGPL-3.0-or-later"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Nomo"
RequestExecutionLevel user
ManifestDPIAware true
ManifestDPIAwareness "PerMonitorV2,System"
SetCompressor /SOLID lzma
!define MUI_ICON "${APPICON}"
!define MUI_UNICON "${APPICON}"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PAYLOAD}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\nomo.exe"
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(NomoDesktopShortcut)"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateNomoDesktopShortcut
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED
!define MUI_PAGE_CUSTOMFUNCTION_SHOW ConfigureNomoFinishPage
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
UninstPage custom un.DataPage un.DataPageLeave
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
LangString NomoCloseForInstall ${LANG_SIMPCHINESE} "是否让 Nomo 退出以继续安装？请在 Nomo 中处理待保存的文档。取消退出将停止安装。$\r$\n$\r$\n$INSTDIR\nomo.exe"
LangString NomoCloseForInstall ${LANG_ENGLISH} "Quit Nomo to continue setup? Follow any save prompts in Nomo. Cancelling quit will stop setup.$\r$\n$\r$\n$INSTDIR\nomo.exe"
LangString NomoQuitPending ${LANG_SIMPCHINESE} "仍在等待 Nomo 退出。请检查 Nomo 中的提示，选择“重试”继续等待，或取消安装。"
LangString NomoQuitPending ${LANG_ENGLISH} "Nomo is still running. Check its prompts, then choose Retry to keep waiting, or cancel setup."
LangString NomoQuitCancelled ${LANG_SIMPCHINESE} "Nomo 未同意退出，安装已停止。"
LangString NomoQuitCancelled ${LANG_ENGLISH} "Nomo did not finish quitting. Setup has stopped."
LangString NomoDowngrade ${LANG_SIMPCHINESE} "已安装 Nomo $NomoInstalledVersion，即将安装较旧的 ${VERSION}。要继续降级吗？"
LangString NomoDowngrade ${LANG_ENGLISH} "Nomo $NomoInstalledVersion is installed. Continue with the downgrade to ${VERSION}?"
LangString NomoDesktopShortcut ${LANG_SIMPCHINESE} "创建桌面快捷方式"
LangString NomoDesktopShortcut ${LANG_ENGLISH} "Create a desktop shortcut"
LangString NomoShortcutFailed ${LANG_SIMPCHINESE} "无法创建桌面快捷方式。你可以从开始菜单打开 Nomo。"
LangString NomoShortcutFailed ${LANG_ENGLISH} "Could not create the desktop shortcut. You can open Nomo from the Start menu."
LangString NomoInstallLocked ${LANG_SIMPCHINESE} "以下文件仍被占用。请从 Nomo 的托盘菜单退出，或关闭占用该文件的程序，然后重试。$\r$\n$\r$\n$INSTDIR\nomo.exe"
LangString NomoInstallLocked ${LANG_ENGLISH} "The file below is still in use. Quit Nomo from its tray menu or close the program using the file, then retry.$\r$\n$\r$\n$INSTDIR\nomo.exe"
LangString NomoDataTitle ${LANG_SIMPCHINESE} "应用数据"
LangString NomoDataTitle ${LANG_ENGLISH} "Application data"
LangString NomoDataSubtitle ${LANG_SIMPCHINESE} "选择卸载后是否保留 Nomo 的设置和记录。"
LangString NomoDataSubtitle ${LANG_ENGLISH} "Choose whether to keep your Nomo settings and history."
LangString NomoDeleteData ${LANG_SIMPCHINESE} "同时清除应用数据"
LangString NomoDeleteData ${LANG_ENGLISH} "Also clear application data"
LangString NomoDataWarning ${LANG_SIMPCHINESE} "清除设置、使用记录、缓存和未保存草稿。保存在其他位置的文档会保留。"
LangString NomoDataWarning ${LANG_ENGLISH} "Clears settings, history, caches and unsaved drafts. Documents saved elsewhere are kept."
LangString NomoDataFailed ${LANG_SIMPCHINESE} "应用数据未能完全清除，卸载已取消。请退出其他 Nomo 进程、检查目录权限后重试。"
LangString NomoDataFailed ${LANG_ENGLISH} "Application data could not be completely cleared. Uninstall was cancelled. Quit other Nomo processes, check folder permissions and retry."
!include /CHARSET=UTF8 "open-with.nsh"
Var NomoUpdateRestart
Var NomoDeleteData
Var NomoDeleteDataCheckbox

Function ConfigureNomoFinishPage
  ${If} $NomoPreviousDir != ""
    ShowWindow $mui.FinishPage.ShowReadme ${SW_HIDE}
  ${EndIf}
FunctionEnd

Function CreateNomoDesktopShortcut
  ClearErrors
  CreateShortcut "$DESKTOP\Nomo.lnk" "$INSTDIR\nomo.exe"
  ${If} ${Errors}
    MessageBox MB_ICONEXCLAMATION "$(NomoShortcutFailed)" /SD IDOK
  ${Else}
    WriteRegDWORD HKCU "${UNINSTKEY}" "DesktopShortcut" 1
  ${EndIf}
FunctionEnd

; Cargo 发布版本采用 SemVer。构建元数据不影响顺序，预发布标识符逐段比较。
Function ConfirmNomoDowngrade
  StrCmp $NomoPreviousDir "" done
  ReadRegStr $NomoInstalledVersion HKCU "${UNINSTKEY}" "DisplayVersion"
  StrCmp $NomoInstalledVersion "" done
  ${StrTok} $0 $NomoInstalledVersion "+" 0 0
  ${StrTok} $1 "${VERSION}" "+" 0 0
  ${StrTok} $2 $0 "-" 0 0
  ${StrTok} $3 $1 "-" 0 0
  ${VersionCompare} $2 $3 $4
  StrCmp $4 1 downgrade
  StrCmp $4 2 done
  StrLen $4 $2
  IntOp $4 $4 + 1
  StrCpy $0 $0 "" $4
  StrLen $4 $3
  IntOp $4 $4 + 1
  StrCpy $1 $1 "" $4
  StrCmp $1 "" done
  StrCmp $0 "" downgrade
  next_identifier:
  ${StrTok} $2 $0 "." 0 0
  ${StrTok} $3 $1 "." 0 0
  StrLen $4 $2
  StrLen $5 $3
  System::Call 'shlwapi::StrSpnW(w r2, w "0123456789") i.r6'
  System::Call 'shlwapi::StrSpnW(w r3, w "0123456789") i.r7'
  ${If} $6 == $4
    StrCmp $7 $5 0 done
    ; 无前导零的数字先比长度，避免大标识符溢出。
    IntCmp $4 $5 compare_text done downgrade
  ${EndIf}
  StrCmp $7 $5 downgrade
  compare_text:
  System::Call 'kernel32::CompareStringOrdinal(w r2, i -1, w r3, i -1, i 0) i.r6'
  StrCmp $6 3 downgrade
  StrCmp $6 2 0 done
  IntOp $4 $4 + 1
  IntOp $5 $5 + 1
  StrCpy $0 $0 "" $4
  StrCpy $1 $1 "" $5
  StrCmp $0 "" done
  StrCmp $1 "" downgrade next_identifier
  downgrade:
  MessageBox MB_ICONEXCLAMATION|MB_YESNO|MB_DEFBUTTON2 "$(NomoDowngrade)" /SD IDNO IDYES done
  SetErrorLevel 1
  Abort
  done:
  ClearErrors
FunctionEnd

Function un.onInit
  StrCpy $NomoDeleteData 0
FunctionEnd

Function un.DataPage
  !insertmacro MUI_HEADER_TEXT "$(NomoDataTitle)" "$(NomoDataSubtitle)"
  nsDialogs::Create 1018
  Pop $0
  StrCmp $0 error 0 +2
    Abort
  ${NSD_CreateCheckbox} 0 0 100% 20u "$(NomoDeleteData)"
  Pop $NomoDeleteDataCheckbox
  ${NSD_SetState} $NomoDeleteDataCheckbox $NomoDeleteData
  ${NSD_CreateLabel} 0 30u 100% 130u "$(NomoDataWarning)"
  Pop $0
  nsDialogs::Show
FunctionEnd

Function un.DataPageLeave
  ${NSD_GetState} $NomoDeleteDataCheckbox $NomoDeleteData
FunctionEnd

; 旧版启动安装器用 /P /R，新版另传已获用户确认退出的进程号。
Function PrepareNomoUpdate
  ${GetParameters} $2
  ${GetOptions} $2 "/NOMOUPDATE=" $3
  IfErrors legacy
  StrCpy $NomoUpdateRestart 1
  SetSilent silent
  IntOp $4 $3 + 0
  StrCmp $4 $3 0 failed
  IntCmpU $4 0 failed failed +1
  System::Call 'kernel32::OpenProcess(i 0x00100000, i 0, i r3) p.r4 ?e'
  Pop $5
  StrCmp $4 0 0 wait_parent
  ; ERROR_INVALID_PARAMETER：父进程已经退出，进程号不再存在。
  StrCmp $5 87 done failed
  wait_parent:
  System::Call 'kernel32::WaitForSingleObject(p r4, i 60000) i.r5'
  System::Call 'kernel32::CloseHandle(p r4)'
  StrCmp $5 0 done failed
  legacy:
  ${GetOptions} $2 "/P" $3
  IfErrors done
  ${GetOptions} $2 "/R" $3
  IfErrors done
  StrCpy $NomoUpdateRestart 1
  SetSilent silent
  IfFileExists "$INSTDIR\nomo.exe" 0 done
  StrCpy $5 240
  legacy_wait:
  ; 没有旧进程号时，只等待已注册安装目录中的二进制可写，不结束其他进程。
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\nomo.exe", i 0x40000000, i 1, p 0, i 3, i 0, p 0) p.r4'
  StrCmp $4 -1 0 legacy_ready
  Sleep 250
  IntOp $5 $5 - 1
  IntCmp $5 0 failed legacy_wait legacy_wait
  legacy_ready:
  System::Call 'kernel32::CloseHandle(p r4)'
  Goto done
  failed:
  MessageBox MB_ICONSTOP "Nomo could not finish closing. The update has not been installed."
  SetErrorLevel 1
  Abort
  done:
  ClearErrors
FunctionEnd

; 通过文件句柄解析实际路径，兼容目录联接和短路径。无法验证的路径返回空值。
Function ResolveNomoImagePath
  System::Store S
  Pop $0
  StrCpy $1 ""
  System::Call 'kernel32::CreateFileW(w r0, i 0, i 7, p 0, i 3, i 0, p 0) p.r2'
  StrCmp $2 -1 done
  System::Call 'kernel32::GetFinalPathNameByHandleW(p r2, w .r1, i ${NSIS_MAX_STRLEN}, i 0) i.r3'
  System::Call 'kernel32::CloseHandle(p r2)'
  ${If} $3 == 0
  ${OrIf} $3 >= ${NSIS_MAX_STRLEN}
    StrCpy $1 ""
  ${EndIf}
  done:
  Push $1
  System::Store L
FunctionEnd

; 返回包含用户 SID 的 TOKEN_USER 缓冲区。调用者负责释放。
Function ReadNomoProcessUser
  System::Store S
  Pop $0
  StrCpy $3 0
  System::Call 'advapi32::OpenProcessToken(p r0, i 8, *p .r1) i.r2'
  StrCmp $2 0 done
  System::Call 'advapi32::GetTokenInformation(p r1, i 1, p 0, i 0, *i .r2)'
  StrCmp $2 0 close_token
  System::Alloc $2
  Pop $3
  StrCmp $3 0 close_token
  System::Call 'advapi32::GetTokenInformation(p r1, i 1, p r3, i r2, *i .r4) i.r5'
  ${If} $5 == 0
    System::Free $3
    StrCpy $3 0
  ${EndIf}
  close_token:
  System::Call 'kernel32::CloseHandle(p r1)'
  done:
  Push $3
  System::Store L
FunctionEnd

; 只匹配当前账户和会话中从目标路径启动的进程，持有句柄直到确认和等待结束。
Function FindNomoInstallProcess
  System::Store S
  StrCpy $7 0
  Push "$INSTDIR\nomo.exe"
  Call ResolveNomoImagePath
  Pop $8
  StrCmp $8 "" done
  System::Call 'kernel32::GetCurrentProcessId() i.r6'
  System::Call 'kernel32::ProcessIdToSessionId(i r6, *i .r9) i.r0'
  StrCmp $0 0 done
  System::Call 'kernel32::GetCurrentProcess() p.s'
  Call ReadNomoProcessUser
  Pop $R0
  StrCmp $R0 0 done
  System::Call '*$R0(p .r11)'
  System::Call 'kernel32::CreateToolhelp32Snapshot(i 2, i 0) p.r1'
  StrCmp $1 -1 free_current_user
  ; NSIS 的 32 位 PROCESSENTRY32W，首字段由 System 插件填入结构大小。
  System::Call '*(&l4, i, i, p, i, i, i, i, i, &w260) p.r2'
  StrCmp $2 0 close_snapshot
  System::Call 'kernel32::Process32FirstW(p r1, p r2) i.r0'
  scan:
  StrCmp $0 0 free_entry
  System::Call '*$2(i, i, i.r3, p, i, i, i, i, i, &w260 .r4)'
  StrCmp $4 "nomo.exe" 0 next
  StrCmp $3 $6 next
  ; PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE
  System::Call 'kernel32::OpenProcess(i 0x00101000, i 0, i r3) p.r7'
  StrCmp $7 0 next
  System::Call 'kernel32::ProcessIdToSessionId(i r3, *i .r5) i.r0'
  StrCmp $0 0 close_process
  StrCmp $5 $9 0 close_process
  System::Call 'kernel32::QueryFullProcessImageNameW(p r7, i 0, w .r4, *i ${NSIS_MAX_STRLEN}) i.r0'
  StrCmp $0 0 close_process
  Push $4
  Call ResolveNomoImagePath
  Pop $4
  StrCmpS $4 $8 0 close_process
  Push $7
  Call ReadNomoProcessUser
  Pop $R2
  StrCmp $R2 0 close_process
  System::Call '*$R2(p .r13)'
  System::Call 'advapi32::EqualSid(p r11, p r13) i.r0'
  System::Free $R2
  StrCmp $0 1 free_entry
  close_process:
  System::Call 'kernel32::CloseHandle(p r7)'
  StrCpy $7 0
  next:
  System::Call 'kernel32::Process32NextW(p r1, p r2) i.r0'
  Goto scan
  free_entry:
  System::Free $2
  close_snapshot:
  System::Call 'kernel32::CloseHandle(p r1)'
  free_current_user:
  System::Free $R0
  done:
  Push $7
  System::Store L
FunctionEnd

Function PrepareNomoManualInstall
  check:
  IfFileExists "$INSTDIR\nomo.exe" 0 done
  ; 先无损探测写权限，避免在文件可覆盖时扫描进程。
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\nomo.exe", i 0x40000000, i 1, p 0, i 3, i 0, p 0) p.r0 ?e'
  Pop $1
  ${If} $0 != -1
    System::Call 'kernel32::CloseHandle(p r0)'
    Goto done
  ${EndIf}

  Call FindNomoInstallProcess
  Pop $0
  ${If} $0 == 0
    ; ERROR_SHARING_VIOLATION / ERROR_USER_MAPPED_FILE。其他写入错误由 NSIS 处理。
    StrCmp $1 32 locked
    StrCmp $1 1224 locked done
  ${EndIf}

  IfSilent close_cancelled
  System::Call 'kernel32::GetProcessId(p r0) i.r3'
  StrCpy $4 0
  find_tray:
  System::Call 'user32::FindWindowExW(p 0, p r4, w "Nomo.Tray", p 0) p.r4'
  StrCmp $4 0 close_locked
  System::Call 'user32::GetWindowThreadProcessId(p r4, *i .r5)'
  StrCmp $5 $3 0 find_tray
  ; 旧版没有退出协议；让用户自行从托盘退出，不发送窗口关闭或强制结束消息。
  System::Call 'user32::SendMessageTimeoutW(p r4, i ${SETUP_QUIT_MESSAGE}, p 0, p r3, i 2, i 1500, *p .r5) p.r6'
  StrCmp $6 0 close_locked
  StrCmp $5 1 0 close_locked
  MessageBox MB_ICONQUESTION|MB_OKCANCEL|MB_DEFBUTTON2 "$(NomoCloseForInstall)" /SD IDCANCEL IDCANCEL close_cancelled
  ; 用户确认期间窗口可能变化。再次核对归属，进程句柄始终保持打开。
  System::Call 'user32::GetWindowThreadProcessId(p r4, *i .r5)'
  StrCmp $5 $3 0 close_locked
  System::Call 'kernel32::CreateEventW(p 0, i 1, i 0, w "Local\Nomo.SetupQuit.$3") p.r7 ?e'
  Pop $6
  StrCmp $7 0 close_locked
  StrCmp $6 183 request_failed ; 另一个安装器已经在等同一进程
  System::Call 'user32::AllowSetForegroundWindow(i r3)'
  System::Call 'user32::SendMessageTimeoutW(p r4, i ${SETUP_QUIT_MESSAGE}, p 1, p r3, i 2, i 1500, *p .r5) p.r6'
  StrCmp $6 0 request_failed
  StrCmp $5 1 0 request_failed
  ; 取消优先。只在目标进程退出后重查文件，等待期间不反复向应用发送退出请求。
  System::Call '*(p r7, p r0) p.r8'
  StrCmp $8 0 request_failed
  wait_quit:
  System::Call 'kernel32::WaitForMultipleObjects(i 2, p r8, i 0, i 30000) i.r2'
  ${If} $2 == 258
    MessageBox MB_ICONINFORMATION|MB_RETRYCANCEL "$(NomoQuitPending)" /SD IDCANCEL IDRETRY wait_quit
  ${EndIf}
  System::Free $8
  System::Call 'kernel32::CloseHandle(p r7)'
  ${If} $2 == 1
    System::Call 'kernel32::GetExitCodeProcess(p r0, *i .r5) i.r6'
    ${If} $6 == 0
    ${OrIf} $5 != 0
      StrCpy $2 0
    ${EndIf}
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
  StrCmp $2 1 check
  ${If} $2 == 0
    SetErrorLevel 1
    Abort "$(NomoQuitCancelled)"
  ${EndIf}
  Goto cancelled
  request_failed:
  System::Call 'kernel32::CloseHandle(p r7)'
  close_locked:
  System::Call 'kernel32::WaitForSingleObject(p r0, i 0) i.r2'
  System::Call 'kernel32::CloseHandle(p r0)'
  StrCmp $2 0 check locked
  close_cancelled:
  System::Call 'kernel32::CloseHandle(p r0)'
  Goto cancelled
  locked:
  MessageBox MB_ICONEXCLAMATION|MB_RETRYCANCEL "$(NomoInstallLocked)" /SD IDCANCEL IDRETRY check
  cancelled:
  SetErrorLevel 1
  Abort
  done:
  ClearErrors
FunctionEnd

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Nomo requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
  ; 沿用既有安装器的卸载键；旧 InstallLocation 带有双引号。
  ReadRegStr $0 HKCU "${UNINSTKEY}" "InstallLocation"
  ${If} $0 != ""
    StrCpy $1 $0 1
    ${If} $1 == '$\"'
      StrCpy $0 $0 -1 1
    ${EndIf}
    StrCpy $INSTDIR $0
    IfFileExists "$0\nomo.exe" 0 +2
      StrCpy $NomoPreviousDir $0
  ${EndIf}
  Call PrepareNomoUpdate
  ${If} $NomoUpdateRestart != 1
    !insertmacro MUI_LANGDLL_DISPLAY
  ${EndIf}
FunctionEnd

Function .onInstSuccess
  ${If} $NomoUpdateRestart == 1
    SetOutPath "$INSTDIR"
    ClearErrors
    Exec '"$INSTDIR\nomo.exe"'
    IfErrors 0 +2
      MessageBox MB_ICONSTOP "Nomo was updated but could not restart. Please open Nomo from the Start menu."
  ${EndIf}
FunctionEnd

Function .onInstFailed
  ${If} $NomoUpdateRestart == 1
    MessageBox MB_ICONSTOP "The Nomo update could not be installed. Close Nomo and try again."
  ${EndIf}
FunctionEnd

Section "Nomo" Main
  Call ConfirmNomoDowngrade
  ; 版本确认和正常退出均通过后才写入安装目录。
  ${If} $NomoUpdateRestart != 1
    Call PrepareNomoManualInstall
  ${EndIf}
  SetOutPath "$INSTDIR"
  ; 文件占用时由安装器报告失败，不能先移除旧版或用户数据。
  ClearErrors
  File "${PAYLOAD}\nomo.exe"
  IfErrors 0 +2
    Abort "Unable to replace nomo.exe. Close Nomo and retry."
  File "${PAYLOAD}\vcruntime140.dll"
  IfErrors 0 +2
    Abort "Unable to replace vcruntime140.dll. Close Nomo and retry."
  File "${PAYLOAD}\LICENSE"
  File "${PAYLOAD}\LICENSE-Lucide"
  File "${PAYLOAD}\LICENSE-STIX"
  File "${PAYLOAD}\LICENSE-Rendering"
  File "${PAYLOAD}\LICENSE-Export"
  File "${PAYLOAD}\LICENSE-Windows"
  SetOutPath "$INSTDIR\samples"
  File "${PAYLOAD}\samples\sample.md"
  SetOutPath "$INSTDIR\samples\assets"
  File "${PAYLOAD}\samples\assets\128x128.png"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  IfErrors 0 +2
    Abort "Unable to write the Nomo install files."
  CreateDirectory "$SMPROGRAMS\Nomo"
  CreateShortcut "$SMPROGRAMS\Nomo\Nomo.lnk" "$INSTDIR\nomo.exe"
  ; 旧安装器总是创建桌面图标，没有所有权标记。升级只接管仍存在的旧图标。
  ReadRegDWORD $0 HKCU "${UNINSTKEY}" "DesktopShortcut"
  IfErrors 0 +2
    StrCpy $0 1
  ${If} $NomoPreviousDir == ""
  ${OrIfNot} ${FileExists} "$DESKTOP\Nomo.lnk"
    StrCpy $0 0
  ${EndIf}
  WriteRegDWORD HKCU "${UNINSTKEY}" "DesktopShortcut" $0
  ${If} $0 == 1
  ${AndIf} $NomoPreviousDir != $INSTDIR
    Call CreateNomoDesktopShortcut
  ${EndIf}
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "Nomo"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "Nomo"
  WriteRegStr HKCU "${UNINSTKEY}" "MainBinaryName" "nomo.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" '$\"$INSTDIR$\"'
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" '$\"$INSTDIR\nomo.exe$\"'
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  !insertmacro NOMO_REGISTER_OPEN_WITH
SectionEnd

Section "Uninstall"
  SetRegView 64
  SetShellVarContext current
  ; 不在应用仍持有文件时继续删除；不强制结束进程或丢弃未保存内容。
  check_uninstall:
  IfFileExists "$INSTDIR\nomo.exe" 0 uninstall_ready
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\nomo.exe", i 0x40000000, i 1, p 0, i 3, i 0, p 0) p.r0'
  StrCmp $0 -1 uninstall_locked
  System::Call 'kernel32::CloseHandle(p r0)'
  Goto uninstall_ready
  uninstall_locked:
  MessageBox MB_ICONEXCLAMATION|MB_RETRYCANCEL "$(NomoInstallLocked)" /SD IDCANCEL IDRETRY check_uninstall
  SetErrorLevel 1
  Abort
  uninstall_ready:
  ${If} $NomoDeleteData == ${BST_CHECKED}
    ; 复用应用的目录校验、跨进程锁和旧版残留清理；卸载不采用开发环境的数据目录。
    SetOutPath "$TEMP"
    System::Call 'kernel32::SetEnvironmentVariableW(w "NOMO_DATA_DIR", p 0)'
    ClearErrors
    ExecWait '"$INSTDIR\nomo.exe" --clear-app-data' $0
    ${If} ${Errors}
    ${OrIf} $0 != 0
      MessageBox MB_ICONSTOP "$(NomoDataFailed)" /SD IDOK
      SetErrorLevel 1
      Abort
    ${EndIf}
  ${EndIf}
  !insertmacro NOMO_UNREGISTER_OPEN_WITH
  Delete "$SMPROGRAMS\Nomo\Nomo.lnk"
  RMDir "$SMPROGRAMS\Nomo"
  ReadRegDWORD $0 HKCU "${UNINSTKEY}" "DesktopShortcut"
  ${If} $0 == 1
    Delete "$DESKTOP\Nomo.lnk"
  ${EndIf}
  Delete "$INSTDIR\nomo.exe"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\LICENSE-Lucide"
  Delete "$INSTDIR\LICENSE-STIX"
  Delete "$INSTDIR\LICENSE-Rendering"
  Delete "$INSTDIR\LICENSE-Export"
  Delete "$INSTDIR\LICENSE-Windows"
  Delete "$INSTDIR\samples\sample.md"
  Delete "$INSTDIR\samples\assets\128x128.png"
  RMDir "$INSTDIR\samples\assets"
  RMDir "$INSTDIR\samples"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTKEY}"
SectionEnd
