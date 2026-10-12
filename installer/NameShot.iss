; NameShot installer for Inno Setup 6.
;
; Built by scripts\make-release.ps1 (preferred -- it stages the payload,
; passes the version from Cargo.toml and drops the Setup.exe into dist\):
;   powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1
;
; Manual build (version falls back to 0.1.0, payload must already be in
; dist\stage\ -- see the [Files] section below):
;   iscc installer\NameShot.iss
;
; Requires Inno Setup 6: https://jrsoftware.org/isdl.php
; No admin rights needed: per-user install into %LocalAppData%\NameShot.

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif
#define MyAppName "NameShot"
#define MyAppExe "nameshot.exe"

[Setup]
AppId={{A98F9DCE-8010-4AFE-93CF-68FE1D3DB342}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=NameShot
DefaultDirName={localappdata}\{#MyAppName}
DefaultGroupName={#MyAppName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename={#MyAppName}-{#MyAppVersion}-win-x64-Setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
VersionInfoVersion={#MyAppVersion}
; Unsigned build: Windows SmartScreen will warn on first run elsewhere.
; See README.md ("Known limitations").

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Payload staged by scripts\make-release.ps1 (same content as the portable zip).
Source: "..\dist\stage\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\stage\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\stage\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#MyAppName}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{autoprograms}\{#MyAppName}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent
