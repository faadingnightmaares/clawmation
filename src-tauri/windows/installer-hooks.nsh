; Keep upgrades in the existing install directory across the accidental
; manufacturer/identifier change introduced after 1.1.1.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R8 SHCTX "Software\a7mda\clawmation" ""
  ReadRegStr $R9 SHCTX "Software\faadingnightmaares\clawmation" ""
  ${If} $R8 == ""
  ${AndIf} $R9 != ""
    StrCpy $INSTDIR $R9
  ${EndIf}
!macroend

; Updater mode normally preserves shortcuts without recreating them. Repair
; every common launch point so it cannot keep opening an older side-by-side
; executable after a successful update.
!macro NSIS_HOOK_POSTINSTALL
  CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"

  ${If} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
    CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}

  ${If} ${FileExists} "$APPDATA\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\${PRODUCTNAME}.lnk"
    !insertmacro SetShortcutTarget "$APPDATA\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$APPDATA\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\${PRODUCTNAME}.lnk"
  ${EndIf}
!macroend
