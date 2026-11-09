; ServerDesk NSIS uninstall hooks.
;
; The uninstaller runs the application's purge mode before removing program
; files so Windows Credential Manager entries can be deleted with the user's
; own profile. If the purge fails (for example because the app is still
; running), the local data directory is kept so a reinstall can retry the
; cleanup, and the user sees an explicit warning.

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing ServerDesk credentials and local data..."
  ExecWait '"$INSTDIR\serverdesk.exe" --purge-data' $0
  StrCmp $0 "0" serverdesk_purge_ok
  MessageBox MB_ICONEXCLAMATION "ServerDesk could not remove all saved credentials from Windows Credential Manager. Local credential references were kept so the cleanup can be retried after reinstalling ServerDesk."
  Goto serverdesk_purge_done

  serverdesk_purge_ok:
  RMDir /r "$APPDATA\com.serverdesk.app"
  RMDir /r "$LOCALAPPDATA\com.serverdesk.app"

  serverdesk_purge_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
