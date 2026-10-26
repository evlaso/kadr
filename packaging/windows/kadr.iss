; kadr's Windows installer (Inno Setup 6), built by scripts/package.sh:
;   iscc /DVersion=0.4.0 /DSource=<dist folder> /DOut=<folder> kadr.iss
; For the user only, no administrator: kadr, its tray and the area overlay
; in %LOCALAPPDATA%\Programs\kadr, a Start menu entry, the tray at login,
; .kadr files opening in kadr, and an uninstaller in Settings → Apps.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\target\dist"
#endif
#ifndef Out
  #define Out "..\..\target\dist"
#endif

[Setup]
AppId={{6B1E3F52-9C0A-4E7D-8B21-4D0F5A7C3E19}
AppName=kadr
AppVersion={#Version}
AppVerName=kadr {#Version}
AppPublisher=kadr
AppPublisherURL=https://github.com/evlaso/kadr
AppSupportURL=https://github.com/evlaso/kadr/issues
DefaultDirName={localappdata}\Programs\kadr
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Out}
OutputBaseFilename=kadr-{#Version}-x86_64-windows-setup
SetupIconFile=..\..\data\windows\kadr.ico
UninstallDisplayIcon={app}\kadr.exe
UninstallDisplayName=kadr
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[Files]
Source: "{#Source}\kadr.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\kadr-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\kadr-overlay.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"

[Icons]
; The capture bar: what kadr opens with from the Start menu.
Name: "{userprograms}\kadr"; Filename: "{app}\kadr.exe"; Parameters: "capture"

[Registry]
; The tray (and with it the hotkeys) at login; kadr writes the same value.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "kadr"; ValueData: """{app}\kadr-tray.exe"""; Flags: uninsdeletevalue
; .kadr projects.
Root: HKCU; Subkey: "Software\Classes\.kadr"; ValueType: string; ValueName: ""; ValueData: "kadr.project"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\kadr.project"; ValueType: string; ValueName: ""; ValueData: "kadr project"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\kadr.project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\kadr.exe,0"
Root: HKCU; Subkey: "Software\Classes\kadr.project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\kadr.exe"" open ""%1"""

[Run]
Filename: "{app}\kadr-tray.exe"; Description: "{cm:LaunchProgram,kadr}"; Flags: nowait postinstall

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im kadr-tray.exe"; Flags: runhidden; RunOnceId: "StopTray"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im kadr.exe"; Flags: runhidden; RunOnceId: "StopKadr"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im kadr-overlay.exe"; Flags: runhidden; RunOnceId: "StopOverlay"

[Code]
// A running kadr holds its files: stop it before they're replaced.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im kadr-tray.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im kadr.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im kadr-overlay.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;
