; Per-user installer for the TRserial desktop app, built by the draft release
; workflow. Pass the version and the folder holding the release files:
;   iscc /DAppVersion=0.1.0 /DSourceDir=<dir> /DOutputDir=<dir> /DOutputName=<name> [/DIconFile=<ico>] trserial.iss
#ifndef AppVersion
  #error Define AppVersion, for example /DAppVersion=0.1.0
#endif
#ifndef SourceDir
  #error Define SourceDir as the folder with trserial-desktop.exe and trserial.exe
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef OutputName
  #define OutputName "trserial-setup"
#endif

[Setup]
; Keep this AppId: Windows uses it to upgrade and uninstall the same app.
AppId={{F6DF1C0A-902B-4C75-9DEC-81148DA92772}
AppName=TRserial
AppVersion={#AppVersion}
AppVerName=TRserial {#AppVersion}
AppPublisher=TRserial
AppPublisherURL=https://github.com/KageRyo/TRserial
AppSupportURL=https://github.com/KageRyo/TRserial/issues
VersionInfoVersion={#AppVersion}
; Installs for the current user only, so no administrator prompt.
PrivilegesRequired=lowest
DefaultDirName={autopf}\TRserial
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern
UninstallDisplayName=TRserial
UninstallDisplayIcon={app}\trserial-desktop.exe
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
#ifdef IconFile
SetupIconFile={#IconFile}
#endif
Compression=lzma2
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
#if FileExists(AddBackslash(CompilerPath) + "Languages\ChineseTraditional.isl")
Name: "chinesetraditional"; MessagesFile: "compiler:Languages\ChineseTraditional.isl"
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\trserial-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\trserial.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\config.example.toml"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\TRserial"; Filename: "{app}\trserial-desktop.exe"
Name: "{autodesktop}\TRserial"; Filename: "{app}\trserial-desktop.exe"; Tasks: desktopicon

[Registry]
; The app writes this value when "start at login" is on. Remove it with the
; program so Windows does not try to start a deleted file. Account data and
; history in the user profile are kept.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "TRserial"; ValueType: none; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\trserial-desktop.exe"; Description: "{cm:LaunchProgram,TRserial}"; Flags: nowait postinstall skipifsilent
