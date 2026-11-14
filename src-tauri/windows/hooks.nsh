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
  StrCmp $1 "Kova" kova_lock kova_other
  kova_lock:
    DetailPrint "Protecting the Kova folder"
    nsExec::ExecToLog 'icacls "$INSTDIR" /inheritance:r /grant:r "*S-1-5-32-544:(OI)(CI)F" "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-545:(OI)(CI)RX" /C /Q'
    Pop $1
    Goto kova_service
  kova_other:
    ; A folder of another name is left as it is. Under Program Files it is protected
    ; already. Anywhere else ordinary programs may change what is in it, and setting the
    ; service up from here would run whatever kova.exe is there as administrator, now and
    ; at every update. There the service is left to Settings, TUN mode, Install, where
    ; Windows asks first.
    StrLen $2 "$PROGRAMFILES64"
    StrCpy $3 "$INSTDIR" $2
    StrCmp $3 "$PROGRAMFILES64" kova_service
    StrLen $2 "$PROGRAMFILES32"
    StrCpy $3 "$INSTDIR" $2
    StrCmp $3 "$PROGRAMFILES32" kova_service
    DetailPrint "The Kova service is not set up from this folder; use Settings, TUN mode"
    Goto kova_service_done
  kova_service:
    DetailPrint "Installing the Kova service for TUN mode"
    nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --service-install'
    Pop $0
    DetailPrint "Kova service setup finished ($0)"
  kova_service_done:
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
