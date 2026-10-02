; Inno Setup script for Pin (per-user install).
; Build:  ISCC /DMyAppVersion=<x.y.z> installer\pin.iss
; MyAppVersion must be numeric (it feeds VersionInfoVersion); the release
; workflow passes the version from Cargo.toml.
; Reuses the legacy MSI UpgradeCode as AppId so existing MSI users upgrade in place.

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

#define MyAppName "Pin"
#define MyAppPublisher "sqhh99"
#define MyAppURL "https://github.com/Sqhh99/pin"
#define MyAppExeName "pin.exe"
; Must match APP_WINDOW_CLASS in src/domain/window.rs.
#define MyAppWindowClass "PinAppMsgWindow"

[Setup]
AppId={{B7F50808-6DBC-48A6-8295-282FB407E038}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
VersionInfoVersion={#MyAppVersion}
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
DisableDirPage=no
; Per-user only: autostart (HKCU Run) and settings (%APPDATA%) are per-user.
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=pin-{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\resource\icon\pin.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile=..\LICENSE
; Pin is asked to exit gracefully in PrepareToInstall; Restart Manager is
; the fallback for versions that predate that (<= 0.1.11).
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"
Name: "autostart"; Description: "Start {#MyAppName} automatically when Windows starts"; GroupDescription: "Startup:"

[Files]
Source: "..\target\x86_64-pc-windows-msvc\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\{#MyAppName}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"
Name: "{userprograms}\{#MyAppName}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Pin"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue; Check: AutoStartOn
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Pin"; Flags: deletevalue uninsdeletevalue; Check: AutoStartOff

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Only Pin's own files: {app} may be a user-chosen folder shared with other files.
Type: files; Name: "{app}\pin.ini"
Type: files; Name: "{userappdata}\Pin\pin.ini"
Type: dirifempty; Name: "{userappdata}\Pin"

[Code]
const
  WM_CLOSE = $0010;
  CloseTimeoutMs = 3000;

function SettingsFile(): String;
begin
  Result := ExpandConstant('{userappdata}\Pin\pin.ini');
end;

// Pre-0.1.12 versions kept pin.ini next to pin.exe.
function LegacySettingsFile(): String;
begin
  Result := ExpandConstant('{app}\pin.ini');
end;

function ExistingSettingsFile(): String;
begin
  Result := '';
  if FileExists(SettingsFile()) then
    Result := SettingsFile()
  else if FileExists(LegacySettingsFile()) then
    Result := LegacySettingsFile();
end;

// Mirrors Settings::parse in src/domain/settings.rs for the AutoStart key.
function ReadAutoStart(const FileName: String): Boolean;
var
  Text: AnsiString;
  S: String;
begin
  Result := False;
  if LoadStringFromFile(FileName, Text) then
  begin
    S := Lowercase(String(Text));
    StringChangeEx(S, ' ', '', True);
    Result := (Pos('autostart=true', S) > 0) or (Pos('autostart=1', S) > 0) or
      (Pos('autostart=yes', S) > 0) or (Pos('autostart=on', S) > 0);
  end;
end;

// The user's autostart choice for this install. Silent upgrades keep the
// current setting (possibly changed from the tray since the last install)
// instead of re-applying the previous install's task selection.
function DesiredAutoStart(): Boolean;
var
  Existing: String;
begin
  Existing := ExistingSettingsFile();
  if WizardSilent() and (Existing <> '') then
    Result := ReadAutoStart(Existing)
  else
    Result := WizardIsTaskSelected('autostart');
end;

function AutoStartOn(): Boolean;
begin
  Result := DesiredAutoStart();
end;

function AutoStartOff(): Boolean;
begin
  Result := not DesiredAutoStart();
end;

procedure CurPageChanged(CurPageID: Integer);
var
  Existing: String;
begin
  // Pre-select the autostart task from the current setting.
  if CurPageID = wpSelectTasks then
  begin
    Existing := ExistingSettingsFile();
    if Existing <> '' then
    begin
      if ReadAutoStart(Existing) then
        WizardSelectTasks('autostart')
      else
        WizardSelectTasks('!autostart');
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Value: String;
begin
  if CurStep = ssPostInstall then
  begin
    if DesiredAutoStart() then
      Value := 'true'
    else
      Value := 'false';
    ForceDirectories(ExpandConstant('{userappdata}\Pin'));
    SaveStringToFile(SettingsFile(), 'AutoStart=' + Value + #13#10, False);
    DeleteFile(LegacySettingsFile());
  end;
end;

function IsPinRunning(): Boolean;
begin
  Result := FindWindowByClassName('{#MyAppWindowClass}') <> 0;
end;

// Ask a running Pin to exit (it unpins windows and removes its tray icon),
// falling back to taskkill if it does not respond in time.
procedure CloseRunningPin();
var
  Waited, ResultCode: Integer;
begin
  if not IsPinRunning() then
    Exit;
  PostMessage(FindWindowByClassName('{#MyAppWindowClass}'), WM_CLOSE, 0, 0);
  Waited := 0;
  while IsPinRunning() and (Waited < CloseTimeoutMs) do
  begin
    Sleep(100);
    Waited := Waited + 100;
  end;
  if IsPinRunning() then
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#MyAppExeName}', '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningPin();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  CloseRunningPin();
  Result := True;
end;
