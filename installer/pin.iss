; Inno Setup script for Pin (per-user install).
; Build:  ISCC /DMyAppVersion=<ver> installer\pin.iss
; Reuses the legacy MSI UpgradeCode as AppId so existing MSI users upgrade in place.

#ifndef MyAppVersion
  #define MyAppVersion "dev"
#endif

#define MyAppName "Pin"
#define MyAppPublisher "sqhh99"
#define MyAppURL "https://github.com/Sqhh99/pin"
#define MyAppExeName "pin.exe"

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
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=dist
OutputBaseFilename=pin-{#MyAppVersion}-windows-x64-setup
SetupIconFile=resource\icon\pin.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile=LICENSE
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"
Name: "autostart"; Description: "Start {#MyAppName} automatically when Windows starts"; GroupDescription: "Startup:"

[Files]
Source: "target\x86_64-pc-windows-msvc\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\{#MyAppName}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"
Name: "{userprograms}\{#MyAppName}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Pin"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue; Tasks: autostart
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Pin"; Flags: deletevalue uninsdeletevalue; Tasks: not autostart

[INI]
Filename: "{app}\pin.ini"; Section: ""; Key: "AutoStart"; String: "true"; Tasks: autostart
Filename: "{app}\pin.ini"; Section: ""; Key: "AutoStart"; String: "false"; Tasks: not autostart

[UninstallDelete]
Type: filesandordirs; Name: "{app}"

[Code]
function InitializeUninstall(): Boolean;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM pin.exe', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  Result := True;
end;
