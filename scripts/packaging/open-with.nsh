!macro NOMO_REGISTER_OPEN_WITH
  ; 注册到 Windows“打开方式”候选列表，但不改写任何文档类型的默认程序。
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe" "FriendlyAppName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\SupportedTypes" ".md" ""
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\SupportedTypes" ".markdown" ""
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\SupportedTypes" ".txt" ""
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\SupportedTypes" ".json" ""
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\shell\open" "" "Open with ${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""

  WriteRegStr SHCTX "Software\Classes\Nomo.Markdown" "" "Markdown Document"
  WriteRegStr SHCTX "Software\Classes\Nomo.Markdown\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\Nomo.Markdown\shell\open" "" "Open with ${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\Nomo.Markdown\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""

  WriteRegStr SHCTX "Software\Classes\Nomo.Text" "" "Text Document"
  WriteRegStr SHCTX "Software\Classes\Nomo.Text\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\Nomo.Text\shell\open" "" "Open with ${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\Nomo.Text\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""

  WriteRegStr SHCTX "Software\Classes\Nomo.Json" "" "JSON Document"
  WriteRegStr SHCTX "Software\Classes\Nomo.Json\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\Nomo.Json\shell\open" "" "Open with ${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\Nomo.Json\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""

  WriteRegStr SHCTX "Software\Classes\.md\OpenWithList\${MAINBINARYNAME}.exe" "" ""
  WriteRegStr SHCTX "Software\Classes\.md\OpenWithProgids" "Nomo.Markdown" ""
  WriteRegStr SHCTX "Software\Classes\.markdown\OpenWithList\${MAINBINARYNAME}.exe" "" ""
  WriteRegStr SHCTX "Software\Classes\.markdown\OpenWithProgids" "Nomo.Markdown" ""
  WriteRegStr SHCTX "Software\Classes\.txt\OpenWithList\${MAINBINARYNAME}.exe" "" ""
  WriteRegStr SHCTX "Software\Classes\.txt\OpenWithProgids" "Nomo.Text" ""
  WriteRegStr SHCTX "Software\Classes\.json\OpenWithList\${MAINBINARYNAME}.exe" "" ""
  WriteRegStr SHCTX "Software\Classes\.json\OpenWithProgids" "Nomo.Json" ""

  ; 在“默认应用”中列出当前用户的 Markdown 候选，不写扩展名默认值或 UserChoice。
  WriteRegStr SHCTX "Software\Nomo\Capabilities" "ApplicationName" "Nomo"
  WriteRegStr SHCTX "Software\Nomo\Capabilities" "ApplicationDescription" "Nomo Markdown 编辑器"
  WriteRegStr SHCTX "Software\Nomo\Capabilities\FileAssociations" ".md" "Nomo.Markdown"
  WriteRegStr SHCTX "Software\Nomo\Capabilities\FileAssociations" ".markdown" "Nomo.Markdown"
  WriteRegStr SHCTX "Software\RegisteredApplications" "Nomo" "Software\Nomo\Capabilities"

  System::Call "shell32::SHChangeNotify(i,i,i,i) (0x08000000, 0x1000, 0, 0)"
!macroend

!macro NOMO_REMOVE_OWN_MENU VERB ARGUMENT
  ReadRegStr $0 SHCTX "Software\Classes\${VERB}\command" ""
  ${If} $0 == '"$INSTDIR\${MAINBINARYNAME}.exe" "${ARGUMENT}"'
  ${OrIf} $0 == '"$INSTDIR\${MAINBINARYNAME}.exe" "${ARGUMENT}\."'
    DeleteRegKey SHCTX "Software\Classes\${VERB}"
  ${EndIf}
!macroend

!macro NOMO_UNREGISTER_OPEN_WITH
  ; 便携版和安装版共用 ProgID 和菜单名，保留已重新绑定到另一份 Nomo 的入口。
  ReadRegStr $0 SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\shell\open\command" ""
  ${If} $0 == '"$INSTDIR\${MAINBINARYNAME}.exe" "%1"'
    DeleteRegKey SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe"
    DeleteRegKey SHCTX "Software\Classes\.md\OpenWithList\${MAINBINARYNAME}.exe"
    DeleteRegKey SHCTX "Software\Classes\.markdown\OpenWithList\${MAINBINARYNAME}.exe"
    DeleteRegKey SHCTX "Software\Classes\.txt\OpenWithList\${MAINBINARYNAME}.exe"
    DeleteRegKey SHCTX "Software\Classes\.json\OpenWithList\${MAINBINARYNAME}.exe"
  ${EndIf}
  ReadRegStr $0 SHCTX "Software\Classes\Nomo.Markdown\shell\open\command" ""
  ${If} $0 == '"$INSTDIR\${MAINBINARYNAME}.exe" "%1"'
    DeleteRegKey SHCTX "Software\Classes\Nomo.Markdown"
    DeleteRegValue SHCTX "Software\Classes\.md\OpenWithProgids" "Nomo.Markdown"
    DeleteRegValue SHCTX "Software\Classes\.markdown\OpenWithProgids" "Nomo.Markdown"
    ReadRegStr $0 SHCTX "Software\RegisteredApplications" "Nomo"
    ${If} $0 == "Software\Nomo\Capabilities"
      DeleteRegValue SHCTX "Software\RegisteredApplications" "Nomo"
      DeleteRegKey SHCTX "Software\Nomo\Capabilities"
    ${EndIf}
  ${EndIf}
  DeleteRegKey SHCTX "Software\Classes\Nomo.Text"
  DeleteRegKey SHCTX "Software\Classes\Nomo.Json"
  DeleteRegValue SHCTX "Software\Classes\.txt\OpenWithProgids" "Nomo.Text"
  DeleteRegValue SHCTX "Software\Classes\.json\OpenWithProgids" "Nomo.Json"
  !insertmacro NOMO_REMOVE_OWN_MENU "SystemFileAssociations\.md\shell\Nomo.Open" "%1"
  !insertmacro NOMO_REMOVE_OWN_MENU "SystemFileAssociations\.markdown\shell\Nomo.Open" "%1"
  !insertmacro NOMO_REMOVE_OWN_MENU "Directory\shell\Nomo.OpenFolder" "%1"
  !insertmacro NOMO_REMOVE_OWN_MENU "Directory\Background\shell\Nomo.OpenFolder" "%V"

  System::Call "shell32::SHChangeNotify(i,i,i,i) (0x08000000, 0x1000, 0, 0)"
!macroend
