; Kova's installer runs elevated (per-machine install), so it can register the Kova
; service, which lets TUN mode work without running Kova itself as administrator.

!include "FileFunc.nsh"

!macro NSIS_HOOK_POSTINSTALL
  ; A folder outside Program Files inherits its drive's rights, and on a second drive
  ; every account may change files there: anyone could then replace kova.exe or put a
  ; DLL beside it, which the service setup runs as administrator. Give Kova's own folder
  ; the rights Program Files has: administrators and SYSTEM change it, users only read.
  ; Only a folder named Kova, so a folder chosen for other things too stays as it is.
  ${GetFileName} "$INSTDIR" $1
  StrCmp $1 "Kova" kova_lock kova_lock_done
  kova_lock:
    DetailPrint "Protecting the Kova folder"
    nsExec::ExecToLog 'icacls "$INSTDIR" /inheritance:r /grant:r "*S-1-5-32-544:(OI)(CI)F" "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-545:(OI)(CI)RX" /C /Q'
    Pop $1
  kova_lock_done:
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
