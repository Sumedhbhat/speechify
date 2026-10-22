; Inno Setup script for the Windows installer. packaging/build.sh runs it
; after PyInstaller, with SPEECHIFY_VERSION set.
;
; Installs per user into %LOCALAPPDATA%\Programs\Speechify without asking for
; admin, adds a Start menu shortcut and the `speechify` command, and updates
; in place when a newer installer is run. Recordings, settings and models in
; %APPDATA%\speechify are never touched, not even by uninstalling.

#define Version GetEnv("SPEECHIFY_VERSION")
#if Version == ""
  #error Set SPEECHIFY_VERSION
#endif

[Setup]
AppId={{6C1F3E0A-8D52-4B8F-9E4A-2F7D1B5C9A31}
AppName=Speechify
AppVersion={#Version}
AppVerName=Speechify {#Version}
AppPublisher=Sumedh Bhat
AppPublisherURL=https://github.com/Sumedhbhat/speechify
AppSupportURL=https://github.com/Sumedhbhat/speechify/issues
DefaultDirName={autopf}\Speechify
DisableDirPage=auto
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=Speechify-{#Version}-windows-x64-setup
SetupIconFile=..\build\icon.ico
UninstallDisplayIcon={app}\Speechify.exe
UninstallDisplayName=Speechify
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
; Closes a running Speechify so an update can replace its files.
CloseApplications=force

[Tasks]
Name: autostart; Description: "Start Speechify when I sign in"; Flags: unchecked

[InstallDelete]
; Files from the previous version (and from the old install.ps1, which used
; the same folder), so nothing stale is left next to the new ones.
Type: filesandordirs; Name: "{app}\_internal"
Type: filesandordirs; Name: "{app}\.venv"
Type: filesandordirs; Name: "{app}\src"
Type: filesandordirs; Name: "{app}\scripts"
Type: filesandordirs; Name: "{app}\docs"
Type: files; Name: "{app}\start.bat"
Type: files; Name: "{app}\start.sh"
Type: files; Name: "{app}\pyproject.toml"
Type: files; Name: "{app}\uv.lock"
Type: files; Name: "{app}\README.md"
Type: files; Name: "{app}\VERSION"

[Files]
Source: "..\build\dist\Speechify\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
; Minimized: the console shows each dictation's latency, and closing it quits.
Name: "{autoprograms}\Speechify"; Filename: "{app}\Speechify.exe"; WorkingDir: "{app}"; Flags: runminimized
Name: "{userstartup}\Speechify"; Filename: "{app}\Speechify.exe"; WorkingDir: "{app}"; Flags: runminimized; Tasks: autostart

[Registry]
; The `speechify` command in new terminals.
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\Speechify.exe"; Description: "Start Speechify"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent runminimized

[Code]
function UserPath(): string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Result) then
    Result := '';
end;

function NeedsAddPath(Dir: string): Boolean;
begin
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(UserPath()) + ';') = 0;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Dir: string;
  P: Integer;
begin
  if CurUninstallStep <> usPostUninstall then
    exit;
  Path := ';' + UserPath() + ';';
  Dir := ';' + ExpandConstant('{app}') + ';';
  P := Pos(Uppercase(Dir), Uppercase(Path));
  if P = 0 then
    exit;
  Delete(Path, P, Length(Dir) - 1);
  Path := Copy(Path, 2, Length(Path) - 2);
  RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);
end;
