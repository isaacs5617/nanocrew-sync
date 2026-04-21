; NanoCrew Sync — NSIS installer hooks
; Silently installs WinFsp before the main app is installed.

!macro customInstall
  ; Install WinFsp silently if the bundled MSI is present
  IfFileExists "$INSTDIR\winfsp.msi" 0 winfsp_skip
    ExecWait 'msiexec /i "$INSTDIR\winfsp.msi" /quiet /norestart'
  winfsp_skip:
!macroend

!macro customUnInstall
  ; WinFsp is a shared driver — do not uninstall it automatically.
!macroend
