Unicode true
Name "Speechify Electron Candidate"
OutFile "speechify-electron-candidate-windows-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\speechify-electron-candidate"
RequestExecutionLevel user
Section
 SetOutPath "$INSTDIR"
 File /r "..\artifacts\speechify-electron-candidate-win32-x64\*"
 CreateShortcut "$SMPROGRAMS\Speechify Electron Candidate.lnk" "$INSTDIR\speechify-electron-candidate.exe"
 WriteUninstaller "$INSTDIR\Uninstall.exe"
SectionEnd
Section "Uninstall"
 Delete "$SMPROGRAMS\Speechify Electron Candidate.lnk"
 RMDir /r "$INSTDIR"
SectionEnd
