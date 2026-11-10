# -*- coding: utf-8 -*-
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"
!define PRODUCTNAME "Nomo"
!define MAINBINARYNAME "nomo"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Nomo"
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
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
LangString NomoCloseForInstall ${LANG_SIMPCHINESE} "此安装目录中的 Nomo 正在运行。请先保存所有文档。$\r$\n点击“确定”后，安装器将结束这个 Nomo 进程，再继续安装。$\r$\n$\r$\n$INSTDIR\nomo.exe"
LangString NomoCloseForInstall ${LANG_ENGLISH} "Nomo is running from this installation folder. Save all your documents first.$\r$\nClick OK to end this Nomo process and continue setup.$\r$\n$\r$\n$INSTDIR\nomo.exe"
LangString NomoInstallLocked ${LANG_SIMPCHINESE} "以下文件仍被占用。请从 Nomo 的托盘菜单退出，或关闭占用该文件的程序，然后重试。$\r$\n$\r$\n$INSTDIR\nomo.exe"
LangString NomoInstallLocked ${LANG_ENGLISH} "The file below is still in use. Quit Nomo from its tray menu or close the program using the file, then retry.$\r$\n$\r$\n$INSTDIR\nomo.exe"
!include /CHARSET=UTF8 "open-with.nsh"
Var NomoUpdateRestart

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
  ; PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE | PROCESS_TERMINATE
  System::Call 'kernel32::OpenProcess(i 0x00101001, i 0, i r3) p.r7'
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

  MessageBox MB_ICONEXCLAMATION|MB_OKCANCEL|MB_DEFBUTTON2 "$(NomoCloseForInstall)" /SD IDCANCEL IDOK close_nomo
  System::Call 'kernel32::CloseHandle(p r0)'
  Goto cancelled
  close_nomo:
  System::Call 'kernel32::TerminateProcess(p r0, i 0)'
  ; 用户可能在确认期间已经自行退出。以等待结果确认进程是否结束。
  System::Call 'kernel32::WaitForSingleObject(p r0, i 10000) i.r2'
  System::Call 'kernel32::CloseHandle(p r0)'
  StrCmp $2 0 check
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
  SetOutPath "$INSTDIR"
  ${If} $NomoUpdateRestart != 1
    Call PrepareNomoManualInstall
  ${EndIf}
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
  CreateShortcut "$DESKTOP\Nomo.lnk" "$INSTDIR\nomo.exe"
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
  !insertmacro NOMO_UNREGISTER_OPEN_WITH
  Delete "$SMPROGRAMS\Nomo\Nomo.lnk"
  RMDir "$SMPROGRAMS\Nomo"
  Delete "$DESKTOP\Nomo.lnk"
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
