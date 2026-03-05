; Nano Snipper — Inno Setup Installer Script
; Requires Inno Setup 6+
; Build: iscc installer\nanosnipper.iss

#define MyAppName "Nano Snipper"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Nano Snipper"
#define MyAppURL "https://github.com/bitcoin-verse/nano-snipper"
#define MyAppExeName "nanosnipper.exe"
#define MySettingsExeName "snipui.exe"

[Setup]
AppId={{E8A3F2B1-7C4D-4E5F-9A1B-3D6C8E2F0A4B}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
DefaultDirName={autopf}\NanoSnipper
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=Output
OutputBaseFilename=NanoSnipperSetup
SetupIconFile=..\resources\app.ico
UninstallDisplayIcon={app}\nanosnipper.exe
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=admin
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "..\target\release\nanosnipper.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\snipui.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{#MyAppName} Settings"; Filename: "{app}\{#MySettingsExeName}"; Parameters: "--page=settings"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{commondesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Remove autostart key on uninstall (the app creates this at runtime)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "NanoSnipper"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent

[Code]
// Kill running instances before install/uninstall
function KillProcess(ExeName: string): Boolean;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ' + ExeName, '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := True;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  KillProcess('nanosnipper.exe');
  KillProcess('snipui.exe');
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    KillProcess('nanosnipper.exe');
    KillProcess('snipui.exe');
  end;
end;
