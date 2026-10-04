; Install and uninstall hooks for the NSIS installer.
;
; Voice tools: speech recognition and the voices (0.7 to 1.5 GB) are not inside the
; installer. When they are missing, the installer offers to fetch them with
; setup-voice.ps1, which it ships, in a window of their own so the download can
; be followed. Every file the script fetches is pinned and checked against its
; SHA-256. A silent or passive install never asks.
;
; This file is included before the installer's languages are loaded, so the
; question picks its text from $LANGUAGE at run time instead of LangStrings.
;
; Uninstall: the app stages frank-hook.exe into %LOCALAPPDATA%\Frank\bin at
; launch, so the installer never recorded it and the default uninstaller leaves
; it behind. The inbox and the log live in the same place and are ours too. The
; "delete the application data" box also takes the settings and the voice tools,
; which Frank keeps under its own name rather than its bundle identifier.
;
; Claude Code's own settings.json is deliberately NOT touched here: it belongs to
; the user, it may contain hooks from other tools, and rewriting somebody's
; config from an uninstaller with no diff and no consent is exactly what the rest
; of this app goes out of its way not to do. A relay that is gone exits 0 without
; printing anything, so a leftover entry costs nothing beyond a dead path.

; The Russian voice's settings file is the last file setup-voice.ps1 always
; fetches, and the script stops at the first failure: with it in place, the
; whole set is.
!macro NSIS_HOOK_POSTINSTALL
  ${IfNot} ${Silent}
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${FileExists} "$LOCALAPPDATA\Frank\voice\piper-voices\ru_RU-dmitri-medium.onnx.json"
    Push $0
    ${If} $LANGUAGE = 1055
      StrCpy $0 "Frank'in seni duyması ve sesli cevap vermesi için ses araçları gerekiyor (konuşma tanıma ve sesler, 0,7 ile 1,5 GB arası). Hepsi ücretsiz ve açık kaynak, senin bilgisayarında çalışır ve sesin dışarı çıkmaz.$\r$\n$\r$\nŞimdi indirilsin mi? İndirme ayrı bir pencerede sürer. Hayır dersen Frank yine çalışır, sadece sesli sohbet olmaz."
    ${ElseIf} $LANGUAGE = 1049
      StrCpy $0 "Чтобы Frank слышал тебя и отвечал голосом, нужны голосовые инструменты (распознавание речи и голоса, от 0,7 до 1,5 ГБ). Они бесплатные и с открытым кодом, работают на твоём компьютере, и твой голос никуда не уходит.$\r$\n$\r$\nСкачать их сейчас? Загрузка пойдёт в отдельном окне. Если нет, Frank всё равно будет работать, только без голосового чата."
    ${Else}
      StrCpy $0 "For Frank to hear you and answer out loud, he needs his voice tools (speech recognition and voices, 0.7 to 1.5 GB). They are free and open source, run on your own computer, and your voice never leaves it.$\r$\n$\r$\nDownload them now? The download runs in its own window. If you say no, Frank still works, just without voice chat."
    ${EndIf}
    MessageBox MB_YESNO|MB_ICONQUESTION "$0" IDNO frank_voice_skip
      Exec '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\setup-voice.ps1" -Pause'
    frank_voice_skip:
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  RMDir /r "$LOCALAPPDATA\Frank\bin"
  RMDir /r "$LOCALAPPDATA\Frank\inbox"
  Delete "$LOCALAPPDATA\Frank\frank.log"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RMDir /r "$APPDATA\Frank"
    RMDir /r "$LOCALAPPDATA\Frank"
  ${EndIf}
!macroend
