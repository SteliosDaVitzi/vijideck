#ifndef AppVersion
  #error AppVersion must be supplied by cargo xtask package
#endif
#ifndef IconPath
  #error IconPath must be supplied by cargo xtask package
#endif

[Setup]
AppId={{B975A203-0375-4E63-9A96-A82D276BC0F2}
AppName=VijiDeck
AppVersion={#AppVersion}
DefaultDirName={autopf}\VijiDeck
DefaultGroupName=VijiDeck
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UsePreviousPrivileges=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
DisableWelcomePage=no
DisableDirPage=no
DisableProgramGroupPage=no
UninstallDisplayIcon={app}\vijideck.ico
SetupIconFile={#IconPath}
OutputDir={#OutputPath}
OutputBaseFilename=vijideck-{#AppVersion}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#IconPath}"; DestDir: "{app}"; DestName: "vijideck.ico"; Flags: ignoreversion

[Icons]
Name: "{group}\VijiDeck"; Filename: "{app}\vijideck.exe"; IconFilename: "{app}\vijideck.ico"
Name: "{autodesktop}\VijiDeck"; Filename: "{app}\vijideck.exe"; IconFilename: "{app}\vijideck.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\vijideck.exe"; Description: "Launch VijiDeck"; Flags: nowait postinstall skipifsilent
