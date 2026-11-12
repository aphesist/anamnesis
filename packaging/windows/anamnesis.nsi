; anamnesis: windows installer (NSIS 3, built on linux by packaging/windows/build.sh)
;
; per-user by default: no administrator rights, installs into
; %LOCALAPPDATA%\Programs\anamnesis, registers the uninstaller under HKCU and never touches
; the brain (%APPDATA%\anamnesis) or the model cache (%LOCALAPPDATA%\anamnesis) unless the
; owner ticks that option in the uninstaller.
;
; two programs: anamnesis-desktop.exe, the desktop app (a native window on microsoft edge
; webview2, no console), which the shortcuts start; and anamnesis.exe, the command line
; for mcp clients and headless use.
;
; makensis -DVERSION=0.1.0 -DSTAGE=<staged files> -DINSTALLED_KB=<size> [-DDISPLAY_VERSION=0.1.0-rc.1]
;          [-DLICENSE_PAGE=<text>] [-DOUTFILE=<setup.exe>] anamnesis.nsi
; build.sh passes all of them.

!ifndef VERSION
  !error "pass -DVERSION=<x.y.z> (numbers only)"
!endif
!ifndef DISPLAY_VERSION
  !define DISPLAY_VERSION "${VERSION}"
!endif
!ifndef LICENSE_PAGE
  !define LICENSE_PAGE "${STAGE}\LICENSE.txt"
!endif
!ifndef INSTALLED_KB
  !define INSTALLED_KB 0
!endif
!ifndef STAGE
  !error "pass -DSTAGE=<folder with the staged files>"
!endif
!ifndef OUTFILE
  !define OUTFILE "anamnesis-${DISPLAY_VERSION}-windows-x86_64-setup.exe"
!endif

Unicode true
Target amd64-unicode
SetCompressor /SOLID lzma
ManifestDPIAware true
RequestExecutionLevel user

!define APP "anamnesis"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\anamnesis"
!define ENV_KEY "Environment"
; webview2 evergreen runtime: its client key, per machine (64-bit view) or per user
!define WEBVIEW2_KEY "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define WEBVIEW2_USER_KEY "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
; microsoft's evergreen bootstrapper (a small download that installs the runtime)
!define WEBVIEW2_BOOTSTRAPPER "https://go.microsoft.com/fwlink/p/?LinkId=2124703"
; the desktop app's own folders (tauri names them after the app identifier): window
; position and webview2's cache
!define APP_ID "ist.aphes.anamnesis"
; nsis strings hold at most NSIS_MAX_STRLEN characters (1024 in the stock build); a longer
; user PATH would be truncated when read, so it is never rewritten
!define /math NSIS_MAX_STRLEN_GUARD ${NSIS_MAX_STRLEN} - 8

Name "${APP}"
Caption "${APP} ${DISPLAY_VERSION}"
BrandingText "${APP} ${DISPLAY_VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\anamnesis"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
ShowInstDetails show
ShowUninstDetails show

; file properties of setup.exe (explorer → properties → details)
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${DISPLAY_VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright 2026 the anamnesis authors. Apache-2.0."

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinMessages.nsh"
!include "StrFunc.nsh"
!include "FileFunc.nsh"
${StrStr}
${UnStrRep}

!define MUI_ICON "${STAGE}\anamnesis.ico"
!define MUI_UNICON "${STAGE}\anamnesis.ico"
!define MUI_ABORTWARNING
; the phi on the aphesis dark ground (build.sh --icon regenerates it)
!define MUI_WELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\wizard.bmp"
!define MUI_COMPONENTSPAGE_NODESC

!define MUI_WELCOMEPAGE_TITLE "anamnesis ${DISPLAY_VERSION}"
!define MUI_WELCOMEPAGE_TEXT "a local-first second brain.$\r$\n$\r$\nthis installs anamnesis for your account only; no administrator rights are needed. your notes and your brain stay on this machine.$\r$\n$\r$\nclick next to continue."
!insertmacro MUI_PAGE_WELCOME
!define MUI_LICENSEPAGE_TEXT_TOP "anamnesis is open source under the apache license 2.0. the notices below also cover the bundled onnx runtime and the aphesis theme assets."
!insertmacro MUI_PAGE_LICENSE "${LICENSE_PAGE}"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\anamnesis-desktop.exe"
!define MUI_FINISHPAGE_RUN_TEXT "open anamnesis now"
!define MUI_FINISHPAGE_TEXT "anamnesis is installed. start it from the start menu; it opens in its own window. the command line anamnesis.exe (for mcp clients) is in the same folder."
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_COMPONENTS
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "anamnesis (required)" SecMain
  SectionIn RO
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${STAGE}\anamnesis-desktop.exe"
  File "${STAGE}\anamnesis.exe"
  File "${STAGE}\WebView2Loader.dll"
  File "${STAGE}\onnxruntime.dll"
  File "${STAGE}\onnxruntime_providers_shared.dll"
  File "${STAGE}\anamnesis.ico"
  File "${STAGE}\LICENSE.txt"
  File "${STAGE}\NOTICE.txt"
  File "${STAGE}\README.txt"
  SetOutPath "$INSTDIR\licenses"
  File /r "${STAGE}\licenses\*.*"
  SetOutPath "$INSTDIR"

  CreateShortCut "$SMPROGRAMS\anamnesis.lnk" "$INSTDIR\anamnesis-desktop.exe" "" "$INSTDIR\anamnesis-desktop.exe" 0 SW_SHOWNORMAL "" "anamnesis: a local-first second brain"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "anamnesis"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${DISPLAY_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "the anamnesis authors"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/aphesist/anamnesis"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\anamnesis.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" ${INSTALLED_KB}

  Call CheckVcRuntime
  Call CheckWebView2
SectionEnd

Section /o "desktop shortcut" SecDesktop
  SetShellVarContext current
  CreateShortCut "$DESKTOP\anamnesis.lnk" "$INSTDIR\anamnesis-desktop.exe" "" "$INSTDIR\anamnesis-desktop.exe" 0 SW_SHOWNORMAL "" "anamnesis"
  WriteRegDWORD HKCU "${UNINST_KEY}" "DesktopShortcut" 1
SectionEnd

Section /o "add to my PATH (terminal, mcp)" SecPath
  ReadRegStr $0 HKCU "${ENV_KEY}" "Path"
  StrLen $1 $0
  ; nsis strings hold at most ${NSIS_MAX_STRLEN} characters: never rewrite a longer PATH
  ${If} $1 >= ${NSIS_MAX_STRLEN_GUARD}
    DetailPrint "your PATH is too long to edit safely here; add $INSTDIR to it yourself"
    MessageBox MB_ICONINFORMATION|MB_OK "your user PATH is too long for the installer to edit safely. add this folder yourself:$\r$\n$INSTDIR" /SD IDOK
    Return
  ${EndIf}
  ${StrStr} $2 ";$0;" ";$INSTDIR;"
  ${If} $2 == ""
    ${If} $0 == ""
      WriteRegExpandStr HKCU "${ENV_KEY}" "Path" "$INSTDIR"
    ${Else}
      WriteRegExpandStr HKCU "${ENV_KEY}" "Path" "$0;$INSTDIR"
    ${EndIf}
    WriteRegDWORD HKCU "${UNINST_KEY}" "AddedToPath" 1
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=2000
    DetailPrint "added $INSTDIR to your user PATH"
  ${EndIf}
SectionEnd

; after the sections, so their ids are defined
Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP|MB_OK "anamnesis needs 64-bit windows (x86_64)." /SD IDOK
    Abort
  ${EndIf}
  SetShellVarContext current
  ; for scripted installs: setup.exe /S /DESKTOP /PATH ticks the two optional components
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/DESKTOP" $R1
  ${IfNot} ${Errors}
    SectionSetFlags ${SecDesktop} ${SF_SELECTED}
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/PATH" $R1
  ${IfNot} ${Errors}
    SectionSetFlags ${SecPath} ${SF_SELECTED}
  ${EndIf}
FunctionEnd

; onnxruntime.dll (microsoft's build) needs the visual c++ runtime. without it anamnesis
; still runs, with the hash embedder, and says why in its log and in settings.
Function CheckVcRuntime
  ; a 64-bit installer sees the real (64-bit) System32
  StrCpy $0 "1"
  ${IfNot} ${FileExists} "$SYSDIR\vcruntime140.dll"
    StrCpy $0 "0"
  ${EndIf}
  ${IfNot} ${FileExists} "$SYSDIR\vcruntime140_1.dll"
    StrCpy $0 "0"
  ${EndIf}
  ${IfNot} ${FileExists} "$SYSDIR\msvcp140.dll"
    StrCpy $0 "0"
  ${EndIf}
  ${If} $0 == "0"
    DetailPrint "the microsoft visual c++ redistributable (x64) is missing: local embeddings fall back to the hash embedder until it is installed"
    MessageBox MB_ICONINFORMATION|MB_YESNO "on-device embeddings need the microsoft visual c++ redistributable (x64), which this pc does not have yet. anamnesis runs without it, with a weaker offline embedder.$\r$\n$\r$\nopen microsoft's download page now?" /SD IDNO IDNO vc_done
    ExecShell "open" "https://aka.ms/vs/17/release/vc_redist.x64.exe"
    vc_done:
  ${EndIf}
FunctionEnd

; the desktop app draws its window with microsoft edge webview2, part of windows 11 and of
; up-to-date windows 10. if it is missing, offer microsoft's evergreen bootstrapper.
Function CheckWebView2
  SetRegView 64
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY}" "pv"
  SetRegView lastused
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ReadRegStr $0 HKCU "${WEBVIEW2_USER_KEY}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    DetailPrint "microsoft edge webview2 runtime not found: the anamnesis window needs it"
    MessageBox MB_ICONEXCLAMATION|MB_YESNO "the anamnesis window needs the microsoft edge webview2 runtime, which this pc does not have yet (windows 11 and updated windows 10 include it).$\r$\n$\r$\ndownload microsoft's installer for it now? run it, then start anamnesis." /SD IDNO IDNO wv_done
    ExecShell "open" "${WEBVIEW2_BOOTSTRAPPER}"
    wv_done:
  ${Else}
    DetailPrint "microsoft edge webview2 runtime $0 found"
  ${EndIf}
FunctionEnd

; ---------------------------------------------------------------- uninstaller

Function un.onInit
  SetShellVarContext current
FunctionEnd

Section "un.anamnesis" UnSecMain
  SectionIn RO
  SetShellVarContext current
  Delete "$SMPROGRAMS\anamnesis.lnk"
  ReadRegDWORD $0 HKCU "${UNINST_KEY}" "DesktopShortcut"
  ${If} $0 == 1
    Delete "$DESKTOP\anamnesis.lnk"
  ${EndIf}

  ReadRegDWORD $0 HKCU "${UNINST_KEY}" "AddedToPath"
  ${If} $0 == 1
    Call un.RemoveFromPath
  ${EndIf}

  Delete /REBOOTOK "$INSTDIR\anamnesis-desktop.exe"
  Delete /REBOOTOK "$INSTDIR\anamnesis.exe"
  Delete /REBOOTOK "$INSTDIR\WebView2Loader.dll"
  Delete /REBOOTOK "$INSTDIR\onnxruntime.dll"
  Delete /REBOOTOK "$INSTDIR\onnxruntime_providers_shared.dll"
  Delete "$INSTDIR\anamnesis.ico"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\NOTICE.txt"
  Delete "$INSTDIR\README.txt"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd

; off by default (and in a silent uninstall): the brain is the owner's data
Section /o "un.also delete my brain and downloaded models" UnSecData
  SetShellVarContext current
  RMDir /r "$APPDATA\anamnesis"
  RMDir /r "$LOCALAPPDATA\anamnesis"
  RMDir /r "$APPDATA\${APP_ID}"
  RMDir /r "$LOCALAPPDATA\${APP_ID}"
  DetailPrint "deleted $APPDATA\anamnesis and $LOCALAPPDATA\anamnesis"
SectionEnd

Function un.RemoveFromPath
  ReadRegStr $0 HKCU "${ENV_KEY}" "Path"
  StrLen $1 $0
  ${If} $1 >= ${NSIS_MAX_STRLEN_GUARD}
    DetailPrint "your PATH is too long to edit safely here; remove $INSTDIR from it yourself"
    Return
  ${EndIf}
  ; work on ";path;" so every entry, first and last included, is delimited
  ${UnStrRep} $2 ";$0;" ";$INSTDIR;" ";"
  ; strip the delimiters added above
  StrCpy $2 $2 "" 1
  StrCpy $2 $2 -1
  ${If} $2 != $0
    WriteRegExpandStr HKCU "${ENV_KEY}" "Path" "$2"
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=2000
    DetailPrint "removed $INSTDIR from your user PATH"
  ${EndIf}
FunctionEnd
