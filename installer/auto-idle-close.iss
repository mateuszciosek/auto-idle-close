; Inno Setup script for the installed flavour of auto-idle-close.
; The portable exe stays the primary artifact; this wraps the very same exe
; with a per-user installer (Start menu shortcut, optional desktop icon and
; autostart, clean uninstall).
;
; Build locally (Inno Setup 6 required):
;   iscc /DAppVersion=0.5.0 installer\auto-idle-close.iss
; CI passes AppVersion from Cargo.toml and expects the release exe at
; target\release\auto-idle-close.exe.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef ExeSource
  #define ExeSource "..\target\release\auto-idle-close.exe"
#endif

[Setup]
AppId={{B7A9E3D4-6C2F-4E8B-9A51-0D7C3F2A8E64}
AppName=Auto Idle Close
AppVersion={#AppVersion}
AppPublisher=Mateusz Ciosek
AppPublisherURL=https://github.com/mateuszciosek/auto-idle-close
AppSupportURL=https://github.com/mateuszciosek/auto-idle-close/issues
; Per-user install, no UAC: config.toml lives next to the exe, so the install
; dir must stay user-writable; autostart and the Explorer verb are HKCU-only.
PrivilegesRequired=lowest
DefaultDirName={autopf}\auto-idle-close
DisableProgramGroupPage=yes
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\auto-idle-close.exe
OutputDir=..\target\installer
OutputBaseFilename=auto-idle-close-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"

[CustomMessages]
; Mirrors autostart_checkbox in src/i18n.rs.
english.AutostartTask=Start with Windows (hidden in the tray)
polish.AutostartTask=Uruchamiaj przy starcie systemu (ukryty w zasobniku)

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked
Name: "autostart"; Description: "{cm:AutostartTask}"; Flags: unchecked

[Files]
Source: "{#ExeSource}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Auto Idle Close"; Filename: "{app}\auto-idle-close.exe"
Name: "{autodesktop}\Auto Idle Close"; Filename: "{app}\auto-idle-close.exe"; Tasks: desktopicon

[Registry]
; Same Run entry the in-app "Start with Windows" checkbox manages, so the
; Settings UI reflects and can toggle what the installer set up.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; ValueName: "auto-idle-close"; \
  ValueData: """{app}\auto-idle-close.exe"" --tray"; \
  Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\auto-idle-close.exe"; Description: "{cm:LaunchProgram,Auto Idle Close}"; \
  Flags: postinstall nowait skipifsilent

[UninstallDelete]
; App-created files next to the exe (portable-style layout).
Type: files; Name: "{app}\config.toml"
Type: files; Name: "{app}\config.toml.tmp"
Type: files; Name: "{app}\launch-queue.txt"

[Code]
// The tray app keeps its exe locked; stop it before replacing or removing it.
procedure KillRunningInstance();
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM auto-idle-close.exe',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  KillRunningInstance();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    KillRunningInstance();
    // HKCU entries the app itself writes (Settings > Integration).
    RegDeleteKeyIncludingSubkeys(HKCU,
      'Software\Classes\exefile\shell\AutoIdleClose');
    RegDeleteValue(HKCU,
      'Software\Microsoft\Windows\CurrentVersion\Run', 'auto-idle-close');
  end;
end;
