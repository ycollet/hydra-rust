Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

!cd ".."
Name "Hydra"
OutFile "dist\hydra-rust-${VERSION}-windows-x86_64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Hydra"
InstallDirRegKey HKCU "Software\Hydra" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "${NUMERIC_VERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "Hydra"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "FileDescription" "Hydra desktop installer"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Hydra contributors"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "Hydra requires 64-bit Windows."
    Abort
  ${EndIf}
FunctionEnd

Section "Hydra"
  SetOutPath "$INSTDIR"
  File "target\x86_64-pc-windows-msvc\release\hydra.exe"
  File "LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Hydra.lnk" "$INSTDIR\hydra.exe"
  WriteRegStr HKCU "Software\Hydra" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hydra" "DisplayName" "Hydra"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hydra" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hydra" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hydra" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hydra" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\Hydra.lnk"
  Delete "$INSTDIR\hydra.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Hydra"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hydra"
SectionEnd
