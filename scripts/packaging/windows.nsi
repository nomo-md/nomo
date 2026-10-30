# -*- coding: utf-8 -*-
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
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
!include "open-with.nsh"

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
  !insertmacro MUI_LANGDLL_DISPLAY
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
  SetOutPath "$INSTDIR\samples"
  File "${PAYLOAD}\samples\sample.md"
  SetOutPath "$INSTDIR\samples\assets"
  File "${PAYLOAD}\samples\assets\128x128.png"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
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
  Delete "$INSTDIR\samples\sample.md"
  Delete "$INSTDIR\samples\assets\128x128.png"
  RMDir "$INSTDIR\samples\assets"
  RMDir "$INSTDIR\samples"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTKEY}"
SectionEnd
