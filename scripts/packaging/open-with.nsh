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

  System::Call "shell32::SHChangeNotify(i,i,i,i) (0x08000000, 0x1000, 0, 0)"
!macroend

!macro NOMO_UNREGISTER_OPEN_WITH
  ; 卸载时只移除 Nomo 自己注册的打开方式入口，保留用户选择的其他默认程序。
  DeleteRegKey SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe"
  DeleteRegKey SHCTX "Software\Classes\Nomo.Markdown"
  DeleteRegKey SHCTX "Software\Classes\Nomo.Text"
  DeleteRegKey SHCTX "Software\Classes\Nomo.Json"
  DeleteRegKey SHCTX "Software\Classes\.md\OpenWithList\${MAINBINARYNAME}.exe"
  DeleteRegValue SHCTX "Software\Classes\.md\OpenWithProgids" "Nomo.Markdown"
  DeleteRegKey SHCTX "Software\Classes\.markdown\OpenWithList\${MAINBINARYNAME}.exe"
  DeleteRegValue SHCTX "Software\Classes\.markdown\OpenWithProgids" "Nomo.Markdown"
  DeleteRegKey SHCTX "Software\Classes\.txt\OpenWithList\${MAINBINARYNAME}.exe"
  DeleteRegValue SHCTX "Software\Classes\.txt\OpenWithProgids" "Nomo.Text"
  DeleteRegKey SHCTX "Software\Classes\.json\OpenWithList\${MAINBINARYNAME}.exe"
  DeleteRegValue SHCTX "Software\Classes\.json\OpenWithProgids" "Nomo.Json"

  System::Call "shell32::SHChangeNotify(i,i,i,i) (0x08000000, 0x1000, 0, 0)"
!macroend
