; Kova's installer runs elevated (per-machine install), so it can register the Kova
; service, which lets TUN mode work without running Kova itself as administrator.

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing the Kova service for TUN mode"
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --service-install'
  Pop $0
  DetailPrint "Kova service setup finished ($0)"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; A Kova that ended without its usual shutdown (also when this uninstaller closed it)
  ; may have left the system proxy pointing at it; with Kova gone, nothing else would
  ; give it back and the PC would have no internet.
  DetailPrint "Restoring the system proxy"
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --restore-proxy'
  Pop $0
  DetailPrint "Removing the Kova service"
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --service-uninstall'
  Pop $0
!macroend
