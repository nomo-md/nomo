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
