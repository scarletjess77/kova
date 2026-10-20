; Kova's installer runs elevated (per-machine install), so it can register the Kova
; service, which lets TUN mode work without running Kova itself as administrator.

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing the Kova service for TUN mode"
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --service-install'
  Pop $0
  DetailPrint "Kova service setup finished ($0)"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing the Kova service"
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --service-uninstall'
  Pop $0
!macroend
