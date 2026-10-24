; The Windows installer (Inno Setup 6), the one way Kurobayomi ships on Windows (plans/addenda/requirements-cross-platform.md,
; packaging). One script for both channels:
;   ISCC /DChannel=stable|nightly /DVersion=X.Y.Z /DBundle=<folder of scripts/bundle-windows.sh> /DIcon=<kurobayomi.ico>
;        /DOutput=<folder> dist\windows\kurobayomi.iss
; It installs for the current user only (no administrator prompt) into %LOCALAPPDATA%\Programs. Stable and nightly have
; their own ids, names and folders, so both can be installed. Only stable takes the kurobayomi:// links and .tachibk
; files, as on Linux, where the nightly's desktop file declares no MIME types.

#if Channel == "nightly"
  #define AppName "Kurobayomi Nightly"
  ; never change: Windows knows an installed app by this id
  #define AppGuid "{{c7d4035a-a6d9-4980-9930-52dbd481e5a5}"
  #define SetupName "Kurobayomi-Nightly-Setup-x86_64"
  #define ShownVersion Version + "-nightly"
#else
  #define AppName "Kurobayomi"
  #define AppGuid "{{b723880d-7007-4b24-8811-a7366f05714c}"
  #define SetupName "Kurobayomi-Setup-x86_64"
  #define ShownVersion Version
#endif

[Setup]
AppId={#AppGuid}
AppName={#AppName}
AppVersion={#ShownVersion}
AppPublisher=Kurobayomi
AppPublisherURL=https://github.com/kurobayomi/kurobayomi
AppSupportURL=https://github.com/kurobayomi/kurobayomi/issues
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern
SetupIconFile={#Icon}
UninstallDisplayIcon={app}\bin\kurobayomi.exe
UninstallDisplayName={#AppName}
; a running Kurobayomi is closed for the update (the self-updater runs the installer silently), and started again after
CloseApplications=yes
RestartApplications=no
#if Channel != "nightly"
ChangesAssociations=yes
#endif
OutputDir={#Output}
OutputBaseFilename={#SetupName}
Compression=lzma2/max
SolidCompression=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; the folders of the previous version first, so no DLL of an older build stays next to the new ones
Type: filesandordirs; Name: "{app}\bin"
Type: filesandordirs; Name: "{app}\lib"
Type: filesandordirs; Name: "{app}\share"
Type: filesandordirs; Name: "{app}\etc"

[Files]
Source: "{#Bundle}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\bin\kurobayomi.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\bin\kurobayomi.exe"; Tasks: desktopicon

#if Channel != "nightly"
[Registry]
; kurobayomi:// links: tracker sign-in returns through one, and the app's own deep links
Root: HKCU; Subkey: "Software\Classes\kurobayomi"; ValueType: string; ValueName: ""; ValueData: "URL:Kurobayomi"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\kurobayomi"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\kurobayomi\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\kurobayomi.exe"",0"
Root: HKCU; Subkey: "Software\Classes\kurobayomi\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\kurobayomi.exe"" ""%1"""
; .tachibk backups open in Kurobayomi
Root: HKCU; Subkey: "Software\Classes\.tachibk"; ValueType: string; ValueName: ""; ValueData: "Kurobayomi.Backup"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\Kurobayomi.Backup"; ValueType: string; ValueName: ""; ValueData: "Kurobayomi backup"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Kurobayomi.Backup\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\kurobayomi.exe"",0"
Root: HKCU; Subkey: "Software\Classes\Kurobayomi.Backup\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\kurobayomi.exe"" ""%1"""
#endif

[Run]
; the finish page's box. The self-updater runs this setup silently when the app quits, and with /RELAUNCH=1 when the
; user pressed Restart, which starts the app again
Filename: "{app}\bin\kurobayomi.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\bin\kurobayomi.exe"; Flags: nowait; Check: Relaunch

[Code]
function Relaunch: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;
